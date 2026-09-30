//! GitHub release updates. All network and filesystem work runs off the UI thread.
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const RELEASE_API: &str = "https://api.github.com/repos/jrmd/dbx/releases/latest";
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Current,
    Available(Update),
    Installing,
    Installed(PathBuf),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Update {
    pub version: Version,
    asset: Asset,
    checksum: Asset,
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("DBX/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()?)
}

fn platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("macos-arm64.zip"),
        ("macos", "x86_64") => Ok("macos-x86_64.zip"),
        ("linux", "x86_64") if appimage().is_some() => Ok("linux-x86_64.AppImage"),
        ("linux", "x86_64") => Ok("linux-x86_64.tar.gz"),
        _ => bail!("Updates are not available for this platform yet."),
    }
}

/// The AppImage file DBX is running from. The AppImage runtime sets
/// `APPIMAGE`; the executable itself lives on a read-only mount, so updates
/// replace this file instead.
fn appimage() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let path = PathBuf::from(std::env::var_os("APPIMAGE")?);
    (path.is_absolute() && path.is_file()).then_some(path)
}

fn select(release: Release, current: &Version, platform: &str) -> Result<Option<Update>> {
    let version = Version::parse(release.tag_name.trim_start_matches('v'))?;
    if release.draft || release.prerelease || !version.pre.is_empty() || version <= *current {
        return Ok(None);
    }
    let name = format!("DBX-{version}-{platform}");
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .context("The latest release does not include a download for this platform yet.")?
        .clone();
    let checksum = release
        .assets
        .iter()
        .find(|asset| asset.name == format!("{name}.sha256"))
        .context("The release is missing its checksum; refusing to install it.")?
        .clone();
    for asset in [&asset, &checksum] {
        let expected = format!(
            "https://github.com/jrmd/dbx/releases/download/{}/{}",
            release.tag_name, asset.name
        );
        ensure!(
            asset.browser_download_url == expected,
            "Unexpected release download URL."
        );
    }
    ensure!(
        asset.size > 0 && asset.size <= MAX_ARCHIVE,
        "Release download is too large or empty."
    );
    ensure!(
        checksum.size > 0 && checksum.size <= 4096,
        "Invalid release checksum size."
    );
    Ok(Some(Update {
        version,
        asset,
        checksum,
    }))
}

pub fn check() -> Result<Option<Update>> {
    let response = client()?
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(30))
        .send()?;
    // A repository with no published release has nothing to update to.
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut json = Vec::new();
    response
        .error_for_status()?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut json)?;
    ensure!(
        json.len() <= 2 * 1024 * 1024,
        "Release metadata is too large."
    );
    select(
        serde_json::from_slice(&json)?,
        &Version::parse(env!("CARGO_PKG_VERSION"))?,
        platform()?,
    )
}

fn expected_checksum(text: &str, name: &str) -> Result<String> {
    let mut words = text.split_whitespace();
    let digest = words.next().context("Empty release checksum.")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA-256 checksum."
    );
    ensure!(
        words.next().map(|s| s.trim_start_matches('*')) == Some(name) && words.next().is_none(),
        "Checksum filename does not match the release."
    );
    Ok(digest.to_ascii_lowercase())
}

fn download(client: &Client, update: &Update, path: &Path) -> Result<()> {
    let mut checksum = String::new();
    client
        .get(&update.checksum.browser_download_url)
        .send()?
        .error_for_status()?
        .take(4097)
        .read_to_string(&mut checksum)?;
    ensure!(checksum.len() <= 4096, "Checksum file is too large.");
    let expected = expected_checksum(&checksum, &update.asset.name)?;
    let mut response = client
        .get(&update.asset.browser_download_url)
        .send()?
        .error_for_status()?;
    let mut file = fs::File::create(path)?;
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        ensure!(
            size <= update.asset.size && size <= MAX_ARCHIVE,
            "Download exceeded the release size."
        );
        digest.update(&buffer[..count]);
        file.write_all(&buffer[..count])?;
    }
    file.sync_all()?;
    ensure!(size == update.asset.size, "Incomplete release download.");
    ensure!(
        format!("{:x}", digest.finalize()) == expected,
        "Release checksum mismatch; the installed app was left untouched."
    );
    Ok(())
}

pub fn install(update: &Update) -> Result<PathBuf> {
    let executable = match appimage() {
        Some(path) => path,
        None => std::env::current_exe()?.canonicalize()?,
    };
    install_at(update, &executable)
}

fn install_at(update: &Update, executable: &Path) -> Result<PathBuf> {
    ensure!(
        !executable.components().any(|c| c.as_os_str() == "target"),
        "Development builds cannot update themselves. Install a release build first."
    );
    #[cfg(target_os = "linux")]
    let destination = executable.to_path_buf();
    #[cfg(target_os = "macos")]
    let destination = app_bundle(executable)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    bail!("This platform cannot install updates.");
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let parent = destination
            .parent()
            .context("Missing installation directory.")?;
        let staging = tempfile::Builder::new().prefix(".dbx-update-").tempdir_in(parent)
            .context("The installation directory is not writable. Move DBX to a user-writable location or update it manually.")?;
        let archive = staging.path().join(&update.asset.name);
        download(&client()?, update, &archive)?;
        #[cfg(target_os = "linux")]
        install_linux(&archive, &destination, staging.path())?;
        #[cfg(target_os = "macos")]
        install_macos(&archive, &destination, staging.path(), &update.version)?;
        Ok(destination)
    }
}

#[cfg(target_os = "linux")]
fn install_linux(archive: &Path, destination: &Path, staging: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if archive.extension().is_some_and(|ext| ext == "AppImage") {
        fs::set_permissions(archive, fs::Permissions::from_mode(0o755))?;
        return replace_linux_executable(archive, destination);
    }
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive)?));
    let replacement = staging.join("dbx");
    let mut found = false;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path != Path::new("./usr/bin/dbx") && path != Path::new("usr/bin/dbx") {
            continue;
        }
        ensure!(
            !found && entry.header().entry_type().is_file(),
            "Invalid release binary entry."
        );
        ensure!(
            entry.size() > 0 && entry.size() <= MAX_ARCHIVE,
            "Invalid release binary size."
        );
        let mut file = fs::File::create(&replacement)?;
        std::io::copy(&mut entry, &mut file)?;
        file.sync_all()?;
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755))?;
        found = true;
    }
    ensure!(found, "The release archive contains no DBX binary.");
    replace_linux_executable(&replacement, destination)
}

#[cfg(target_os = "linux")]
fn replace_linux_executable(replacement: &Path, destination: &Path) -> Result<()> {
    // Reject wrong-architecture or broken binaries before replacing the running executable.
    let mut header = [0; 20];
    fs::File::open(replacement)?.read_exact(&mut header)?;
    ensure!(
        &header[..4] == b"\x7fELF" && header[4] == 2 && header[5] == 1 && header[18..20] == [62, 0],
        "The release binary is not Linux x86_64."
    );
    fs::rename(replacement, destination)
        .context("Could not replace DBX; the installed app was left untouched.")?;
    fs::File::open(
        destination
            .parent()
            .context("Missing installation directory")?,
    )?
    .sync_all()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn app_bundle(executable: &Path) -> Result<PathBuf> {
    let app = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("Updates require an installed DBX.app bundle.")?;
    ensure!(
        app.file_name().is_some_and(|n| n == "DBX.app")
            && executable == app.join("Contents/MacOS/dbx"),
        "Updates require an installed DBX.app bundle."
    );
    ensure!(
        !app.starts_with("/Volumes"),
        "Drag DBX to Applications before updating."
    );
    Ok(app.to_path_buf())
}

#[cfg(target_os = "macos")]
fn command(program: &str, args: &[&std::ffi::OsStr]) -> Result<std::process::Output> {
    let output = std::process::Command::new(program).args(args).output()?;
    ensure!(
        output.status.success(),
        "{program} rejected the update: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

#[cfg(target_os = "macos")]
fn install_macos(
    archive: &Path,
    destination: &Path,
    staging: &Path,
    version: &Version,
) -> Result<()> {
    use std::ffi::OsStr;
    let signature = command(
        "/usr/bin/codesign",
        &[
            OsStr::new("-dv"),
            OsStr::new("--verbose=4"),
            destination.as_os_str(),
        ],
    )?;
    let signature = String::from_utf8_lossy(&signature.stderr);
    ensure!(
        signature.contains("Authority=Developer ID Application:"),
        "Only Developer ID release builds can update themselves."
    );
    let team = signature
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .context("The installed app has no signing team.")?;
    ensure!(
        team.len() == 10 && team.bytes().all(|b| b.is_ascii_alphanumeric()),
        "Invalid signing team."
    );
    let extracted = staging.join("extracted");
    command(
        "/usr/bin/ditto",
        &[
            OsStr::new("-x"),
            OsStr::new("-k"),
            archive.as_os_str(),
            extracted.as_os_str(),
        ],
    )?;
    let replacement = extracted.join("DBX.app");
    let requirement = format!(
        "anchor apple generic and identifier \"dev.jrmd.dbx\" and certificate leaf[subject.OU] = \"{team}\""
    );
    command(
        "/usr/bin/codesign",
        &[
            OsStr::new("--verify"),
            OsStr::new("--deep"),
            OsStr::new("--strict"),
            OsStr::new("-R"),
            OsStr::new(&requirement),
            replacement.as_os_str(),
        ],
    )?;
    command(
        "/usr/sbin/spctl",
        &[
            OsStr::new("--assess"),
            OsStr::new("--type"),
            OsStr::new("execute"),
            replacement.as_os_str(),
        ],
    )?;
    let plist = replacement.join("Contents/Info.plist");
    let bundle_version = command(
        "/usr/libexec/PlistBuddy",
        &[
            OsStr::new("-c"),
            OsStr::new("Print :CFBundleShortVersionString"),
            plist.as_os_str(),
        ],
    )?;
    ensure!(
        String::from_utf8_lossy(&bundle_version.stdout).trim() == version.to_string(),
        "Bundle version does not match the release."
    );
    swap_bundles(&replacement, destination)?;
    Ok(())
}

/// AppKit bundles need an atomic directory exchange: a power loss must not
/// leave Applications without DBX between two separate rename operations.
#[cfg(target_os = "macos")]
fn swap_bundles(replacement: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let replacement = CString::new(replacement.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // SAFETY: both C strings are valid and live for the syscall. Both paths
    // are on the same filesystem because staging is in the app's parent.
    let result = unsafe {
        libc::renamex_np(
            replacement.as_ptr(),
            destination.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("Could not replace DBX; the installed app was left untouched.");
    }
    Ok(())
}

/// Restart only after an explicit click. Shell arguments remain positional,
/// so spaces and metacharacters in installation paths are never interpreted.
pub fn restart(destination: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    let (program, argument) = (destination.to_path_buf(), None::<PathBuf>);
    #[cfg(target_os = "macos")]
    let (program, argument) = (
        PathBuf::from("/usr/bin/open"),
        Some(destination.to_path_buf()),
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", "sleep 2; exec \"$@\"", "dbx-restart"])
            .arg(program);
        if let Some(argument) = argument {
            command.arg("-n").arg(argument);
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    bail!("Restart DBX manually.")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(version: &str) -> Release {
        let name = format!("DBX-{version}-linux-x86_64.tar.gz");
        Release {
            tag_name: format!("v{version}"),
            draft: false,
            prerelease: false,
            assets: [name.clone(), format!("{name}.sha256")]
                .into_iter()
                .map(|name| Asset {
                    browser_download_url: format!(
                        "https://github.com/jrmd/dbx/releases/download/v{version}/{name}"
                    ),
                    name,
                    size: 100,
                })
                .collect(),
        }
    }
    #[test]
    fn selects_only_new_stable_matching_assets() {
        let current = Version::parse("0.1.0").unwrap();
        assert!(
            select(release("0.1.0"), &current, "linux-x86_64.tar.gz")
                .unwrap()
                .is_none()
        );
        assert!(
            select(release("0.0.9"), &current, "linux-x86_64.tar.gz")
                .unwrap()
                .is_none()
        );
        assert!(
            select(release("0.2.0-beta.1"), &current, "linux-x86_64.tar.gz")
                .unwrap()
                .is_none()
        );
        let mut draft = release("0.2.0");
        draft.draft = true;
        assert!(
            select(draft, &current, "linux-x86_64.tar.gz")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            select(release("0.2.0"), &current, "linux-x86_64.tar.gz")
                .unwrap()
                .unwrap()
                .version
                .to_string(),
            "0.2.0"
        );
        assert!(select(release("0.2.0"), &current, "macos-arm64.zip").is_err());
    }
    #[test]
    fn rejects_missing_checksum_and_foreign_downloads() {
        let current = Version::parse("0.1.0").unwrap();
        let mut missing = release("0.2.0");
        missing.assets.pop();
        assert!(select(missing, &current, "linux-x86_64.tar.gz").is_err());
        let mut foreign = release("0.2.0");
        foreign.assets[0].browser_download_url = "https://example.test/dbx".into();
        assert!(select(foreign, &current, "linux-x86_64.tar.gz").is_err());
        assert!(
            expected_checksum(&format!("{}  wrong.tar.gz", "a".repeat(64)), "dbx.tar.gz").is_err()
        );
        assert!(expected_checksum("bad  dbx.tar.gz", "dbx.tar.gz").is_err());
        assert_eq!(
            expected_checksum(&format!("{}  dbx.tar.gz", "A".repeat(64)), "dbx.tar.gz").unwrap(),
            "a".repeat(64)
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_bundle_exchange_preserves_both_directories() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("DBX.app");
        let new = dir.path().join("replacement.app");
        fs::create_dir(&old).unwrap();
        fs::create_dir(&new).unwrap();
        fs::write(old.join("version"), "old").unwrap();
        fs::write(new.join("version"), "new").unwrap();
        swap_bundles(&new, &old).unwrap();
        assert_eq!(fs::read(old.join("version")).unwrap(), b"new");
        assert_eq!(fs::read(new.join("version")).unwrap(), b"old");
        assert!(swap_bundles(&dir.path().join("missing"), &old).is_err());
        assert_eq!(fs::read(old.join("version")).unwrap(), b"new");
    }

    #[test]
    fn download_checks_the_actual_bytes_and_size() {
        use std::net::TcpListener;
        fn attempt(bytes: &[u8], expected: &[u8], advertised: u64) -> Result<()> {
            let server = TcpListener::bind("127.0.0.1:0")?;
            let base = format!("http://{}", server.local_addr()?);
            let checksum = format!("{:x}  test.tar.gz", Sha256::digest(expected));
            let responses = [checksum.into_bytes(), bytes.to_vec()];
            let worker = std::thread::spawn(move || {
                for body in responses {
                    let (mut stream, _) = server.accept().unwrap();
                    let mut request = [0; 4096];
                    assert!(stream.read(&mut request).unwrap() > 0);
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(&body).unwrap();
                }
            });
            let update = Update {
                version: Version::parse("0.2.0")?,
                asset: Asset {
                    name: "test.tar.gz".into(),
                    browser_download_url: format!("{base}/archive"),
                    size: advertised,
                },
                checksum: Asset {
                    name: "test.tar.gz.sha256".into(),
                    browser_download_url: format!("{base}/checksum"),
                    size: 100,
                },
            };
            let dir = tempfile::tempdir()?;
            // Production uses HTTPS-only. This isolated fixture exercises the
            // identical streaming/hash path against a loopback HTTP server.
            let result = download(
                &Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(5))
                    .build()?,
                &update,
                &dir.path().join("archive"),
            );
            worker.join().unwrap();
            result
        }
        assert!(attempt(b"valid", b"valid", 5).is_ok());
        assert!(attempt(b"wrong", b"valid", 5).is_err());
        assert!(attempt(b"short", b"short", 10).is_err());
        assert!(attempt(b"oversized", b"oversized", 5).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_install_replaces_atomically_and_preserves_original_on_invalid_archive() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("installed");
        fs::write(&destination, "original").unwrap();
        let archive = dir.path().join("update.tar.gz");
        let make_archive = |bytes: &[u8], entry_type| {
            let encoder = flate2::write::GzEncoder::new(
                fs::File::create(&archive).unwrap(),
                flate2::Compression::default(),
            );
            let mut tar = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_entry_type(entry_type);
            header.set_cksum();
            tar.append_data(&mut header, "./usr/bin/dbx", bytes)
                .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        };
        make_archive(b"not an ELF executable", tar::EntryType::Regular);
        assert!(install_linux(&archive, &destination, dir.path()).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        let mut bytes = vec![0; 64];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[18] = 62;
        make_archive(&bytes, tar::EntryType::Regular);
        install_linux(&archive, &destination, dir.path()).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), bytes);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn appimage_update_replaces_the_appimage_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("DBX.AppImage");
        fs::write(&destination, "original").unwrap();
        let download = dir.path().join("DBX-9.9.9-linux-x86_64.AppImage");
        fs::write(&download, "not an ELF executable").unwrap();
        assert!(install_linux(&download, &destination, dir.path()).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        let mut bytes = vec![0; 64];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[18] = 62;
        fs::write(&download, &bytes).unwrap();
        install_linux(&download, &destination, dir.path()).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        let mode = fs::metadata(&destination).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}
