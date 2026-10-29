#!/usr/bin/env python3
"""Build the macOS release locally; CI builds Linux and publishes the release."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
REPO = "jrmd/dbx"
RELEASES = ROOT / "target/releases"


def run(*args, cwd=ROOT, env=None, capture=False, log=None):
    result = subprocess.run(
        list(map(str, args)), cwd=cwd, env=env, text=True,
        stdout=subprocess.PIPE if capture else log,
        stderr=subprocess.PIPE if capture else (subprocess.STDOUT if log else None),
        check=True,
    )
    return result.stdout.strip() if capture else None


def version_tag(tag=None):
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version):
        raise ValueError("Only stable MAJOR.MINOR.PATCH versions can be released.")
    if tag and tag != f"v{version}":
        raise ValueError("TAG must match the workspace version in Cargo.toml.")
    return version, f"v{version}"


def source_commit(tag, clean=False):
    if clean and run("git", "status", "--porcelain", "--untracked-files=normal", capture=True):
        raise ValueError("Commit the release changes first, or use a clean worktree. The working tree is dirty.")
    commit = run("git", "rev-parse", f"refs/tags/{tag}^{{commit}}", capture=True)
    if clean and commit != run("git", "rev-parse", "HEAD", capture=True):
        raise ValueError(f"Check out {tag} before building: its commit must match HEAD.")
    return commit


def doctor():
    problems = []
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        problems.append("Use an Apple Silicon Mac for the macOS arm64 release.")
    for command in ("git", "cargo", "rustc", "gh", "xcrun", "cmake"):
        if not shutil.which(command):
            problems.append(f"Missing command: {command}")
    checks = [
        ("GitHub login", ("gh", "auth", "status")),
        ("Metal compiler", ("xcrun", "--find", "metal")),
        ("Rust formatting component", ("cargo", "fmt", "--version")),
        ("Rust Clippy component", ("cargo", "clippy", "--version")),
        ("Developer ID and notarization", ("bash", "scripts/release-macos.sh", "check-local")),
    ]
    for label, command in checks:
        try:
            run(*command, capture=True)
            print(f"OK: {label}", flush=True)
        except (OSError, subprocess.CalledProcessError) as error:
            detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) else str(error)
            problems.append(f"{label}: {detail}")
    if problems:
        raise ValueError("\n".join(problems))


def asset_names(version):
    return [f"DBX-{version}-macos-arm64.zip"]


def linux_asset_names(version):
    return [f"DBX-{version}-linux-x86_64.tar.gz", f"DBX-{version}-linux-x86_64.AppImage"]


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def verify_assets(directory, version):
    for name in asset_names(version):
        asset = directory / name
        if not asset.is_file() or asset.stat().st_size == 0:
            raise ValueError(f"Missing or empty release asset: {asset}")
        checksum = (directory / (name + ".sha256")).read_text().strip()
        if checksum != f"{digest(asset)}  {name}":
            raise ValueError(f"Invalid checksum or filename for {name}")


def manifest_files(version):
    return [item for name in asset_names(version) for item in (name, name + ".sha256")] + ["release-notes.md"]


def verify_manifest(directory, version, tag, commit):
    manifest = json.loads((directory / "manifest.json").read_text())
    if (manifest.get("format"), manifest.get("tag"), manifest.get("commit")) != (1, tag, commit):
        raise ValueError("Release manifest does not match the selected tag and commit. Rebuild it.")
    expected = manifest_files(version)
    if set(manifest.get("sha256", {})) != set(expected):
        raise ValueError("Release manifest must contain exactly the expected assets and notes.")
    for name in expected:
        if digest(directory / name) != manifest["sha256"][name]:
            raise ValueError(f"Release file changed after validation: {name}")
    verify_assets(directory, version)
    return manifest


def build(version, tag):
    commit = source_commit(tag, clean=True)
    doctor()
    rust = run("rustc", "--version", capture=True).split()[1]
    if not re.fullmatch(r"\d+\.\d+\.\d+", rust):
        raise ValueError("Use a stable Rust compiler for releases.")
    output = RELEASES / tag
    output.mkdir(parents=True, exist_ok=True)
    (output / "manifest.json").unlink(missing_ok=True)
    # Build a frozen tracked snapshot: ongoing edits cannot enter the release.
    with tempfile.TemporaryDirectory(prefix=".source-", dir=RELEASES) as temporary:
        source = Path(temporary)
        archive = source / "source.tar"
        run("git", "archive", "--format=tar", "--output", archive, commit)
        run("tar", "-xf", archive, "-C", source)
        archive.unlink()
        run("python3", "scripts/release-notes.py", tag, "--output", output / "release-notes.md", cwd=source)
        for test in ("test-macos-release.py", "test-build-profiles.py", "test-release-notes.py", "test-local-release.py"):
            run("python3", source / "scripts" / test, cwd=source)
        logfile = output / "macos-build.log"
        print(f"Building macOS; log: {logfile}", flush=True)
        env = os.environ.copy()
        # A dedicated persistent cache prevents simultaneous development
        # builds from replacing the binary between compilation and signing.
        env.update(CARGO_TARGET_DIR=str(ROOT / "target/release-macos-cache"), DBX_CARGO_PROFILE="release")
        env.pop("CARGO_BUILD_TARGET", None)
        try:
            with logfile.open("w") as log:
                run("cargo", "fmt", "--all", "--", "--check", cwd=source, env=env, log=log)
                run("cargo", "test", "--locked", "--release", "--workspace", cwd=source, env=env, log=log)
                run("cargo", "clippy", "--locked", "--release", "--workspace", "--all-targets",
                    "--", "-D", "warnings", cwd=source, env=env, log=log)
                run("bash", "scripts/release-macos.sh", "local", cwd=source, env=env, log=log)
                env["DBX_MACOS_UPDATE_ARCHIVE"] = str(source / "target/macos" / asset_names(version)[0])
                run("cargo", "test", "--locked", "--release", "-p", "dbx-ui",
                    "signed_macos_archive_installs_and_preserves_app_on_rejection",
                    "--", "--ignored", "--nocapture", cwd=source, env=env, log=log)
        except subprocess.CalledProcessError as error:
            raise ValueError(f"Build failed; inspect {logfile}.") from error
        for suffix in ("", ".sha256"):
            shutil.copy2(source / "target/macos" / (asset_names(version)[0] + suffix), output)
        print("macOS build and signed updater verification passed.", flush=True)
        verify_assets(output, version)
        manifest = {
            "format": 1, "tag": tag, "commit": commit, "rust": rust,
            "sha256": {name: digest(output / name) for name in manifest_files(version)},
        }
        (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Verified release ready in {output}\nSmoke-test the Mac app, then run: make release-publish TAG={tag}\nPublishing uploads the Mac build; CI then builds Linux and publishes the release.")


def release_info(tag):
    try:
        return json.loads(run("gh", "api", f"repos/{REPO}/releases/tags/{tag}", capture=True))
    except subprocess.CalledProcessError as error:
        if "HTTP 404" not in (error.stderr or ""):
            raise
    # GitHub's release-by-tag endpoint omits drafts. The authenticated list
    # includes them, including drafts beyond its first page.
    pages = json.loads(run("gh", "api", "--paginate", "--slurp",
                           f"repos/{REPO}/releases?per_page=100", capture=True))
    return next((info for page in pages for info in page if info.get("tag_name") == tag), None)


def require_draft(info):
    if info is not None and not info.get("draft"):
        raise ValueError("This release is already published; published assets are never overwritten.")


def verify_remote_tag(tag, commit):
    remote = run("git", "ls-remote", "origin", f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}", capture=True)
    refs = dict(line.split()[::-1] for line in remote.splitlines())
    remote_commit = refs.get(f"refs/tags/{tag}^{{}}", refs.get(f"refs/tags/{tag}"))
    if remote_commit and remote_commit != commit:
        raise ValueError("The remote release tag points to another commit; refusing to publish.")
    return remote_commit


def publish(version, tag):
    commit = source_commit(tag)
    output = RELEASES / tag
    manifest = verify_manifest(output, version, tag, commit)
    origin = run("git", "remote", "get-url", "origin", capture=True)
    if origin not in (f"git@github.com:{REPO}.git", f"https://github.com/{REPO}.git",
                      f"https://github.com/{REPO}", f"ssh://git@github.com/{REPO}.git"):
        raise ValueError(f"origin must point to {REPO}; refusing to push a release tag elsewhere.")
    info = release_info(tag)
    require_draft(info)
    if not verify_remote_tag(tag, commit):
        run("git", "push", "origin", f"refs/tags/{tag}:refs/tags/{tag}")
    verify_remote_tag(tag, commit)
    if info is None:
        run("gh", "release", "create", tag, "--repo", REPO, "--verify-tag", "--draft",
            "--title", tag, "--notes-file", output / "release-notes.md")
    info = release_info(tag)
    if info is None:
        raise ValueError("The draft release could not be read.")
    require_draft(info)
    names = [name for name in manifest_files(version) if name != "release-notes.md"]
    # Linux assets may already be present when a draft is resumed after CI ran.
    allowed = set(names) | {item for name in linux_asset_names(version) for item in (name, name + ".sha256")}
    if set(asset["name"] for asset in info["assets"]) - allowed:
        raise ValueError("Draft contains unexpected assets; inspect it before retrying.")
    run("gh", "release", "edit", tag, "--repo", REPO, "--notes-file", output / "release-notes.md")
    run("gh", "release", "upload", tag, *(output / name for name in names),
        "--repo", REPO, "--clobber")
    # Verify the bytes GitHub actually received before exposing the release to updaters.
    with tempfile.TemporaryDirectory(prefix=".download-", dir=RELEASES) as temporary:
        run("gh", "release", "download", tag, "--repo", REPO, "--dir", temporary,
            *(arg for name in names for arg in ("--pattern", name)))
        downloaded = Path(temporary)
        if set(path.name for path in downloaded.iterdir()) != set(names):
            raise ValueError("Uploaded release asset set does not match the build.")
        for name in names:
            if digest(downloaded / name) != manifest["sha256"][name]:
                raise ValueError(f"Uploaded asset failed verification: {name}")
    final_info = release_info(tag)
    if final_info is None:
        raise ValueError("The draft disappeared during upload; refusing to publish.")
    require_draft(final_info)
    if not verify_remote_tag(tag, commit):
        raise ValueError("The remote tag disappeared during upload; refusing to publish.")
    # Mac ships first: the verified Mac build goes out now, and CI attaches the
    # Linux packages to the published release when they are ready.
    run("gh", "release", "edit", tag, "--repo", REPO, "--draft=false", "--latest")
    run("gh", "workflow", "run", "linux-release.yml", "--repo", REPO, "--ref", tag)
    print(f"Published https://github.com/{REPO}/releases/tag/{tag}\n"
          f"CI is building Linux and will add it: https://github.com/{REPO}/actions/workflows/linux-release.yml")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("doctor", "build", "publish"))
    parser.add_argument("--tag", help="Must match Cargo.toml; defaults to v<workspace version>.")
    args = parser.parse_args()
    RELEASES.mkdir(parents=True, exist_ok=True)
    with (RELEASES / ".lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            parser.exit(1, "Another local release operation is already running.\n")
        try:
            if args.action == "doctor":
                doctor()
            else:
                version, tag = version_tag(args.tag)
                (build if args.action == "build" else publish)(version, tag)
        except (ValueError, OSError, subprocess.CalledProcessError) as error:
            detail = error.stderr if isinstance(error, subprocess.CalledProcessError) and error.stderr else str(error)
            parser.exit(1, f"Release stopped: {detail}\n")


if __name__ == "__main__":
    main()
