#!/usr/bin/env python3
"""Validate an Apple-issued certificate against a local key and install CI secrets.

Private material stays outside the repository. Secret values go to gh over stdin.
"""
import argparse
import base64
import os
from pathlib import Path
import re
import secrets
import subprocess
import urllib.request


def run(*args, data=None, env=None):
    return subprocess.check_output(args, input=data, env=env, stderr=subprocess.PIPE)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("certificate", type=Path)
    parser.add_argument("--team", required=True)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--repo", required=True)
    args = parser.parse_args()
    os.umask(0o077)
    directory = args.directory.resolve()
    key = directory / "private-key.pem"
    if not key.is_file():
        raise SystemExit("Missing local signing key; use the key that created the CSR.")
    certificate = run("openssl", "x509", "-inform", "DER", "-in", str(args.certificate))
    cert = directory / "developer-id.pem"
    cert.write_bytes(certificate)
    text = run("openssl", "x509", "-in", str(cert), "-noout", "-text").decode()
    subject = run("openssl", "x509", "-in", str(cert), "-noout", "-subject", "-nameopt", "multiline").decode()
    if not re.search(r"organizationalUnitName\s*=\s*" + re.escape(args.team) + r"\s*$", subject, re.M):
        raise SystemExit("Certificate belongs to a different Apple team.")
    if "1.2.840.113635.100.6.1.13:" not in text:
        raise SystemExit("Certificate is not a Developer ID Application certificate.")
    run("openssl", "x509", "-in", str(cert), "-noout", "-checkend", "0")
    public_cert = run("openssl", "x509", "-in", str(cert), "-noout", "-pubkey")
    public_key = run("openssl", "pkey", "-in", str(key), "-pubout")
    if public_cert != public_key:
        raise SystemExit("Certificate does not match the private key used for our CSR.")
    # Include Apple's intermediate so a fresh runner can construct the trust chain.
    issuer = run("openssl", "x509", "-in", str(cert), "-noout", "-issuer", "-nameopt", "RFC2253").decode().strip().split("=", 1)[1]
    intermediate = directory / "intermediate.pem"
    filename = "DeveloperIDG2CA.cer" if "OU=G2" in issuer else "DeveloperIDCA.cer"
    with urllib.request.urlopen("https://www.apple.com/certificateauthority/" + filename, timeout=30) as response:
        der = response.read()
    intermediate.write_bytes(run("openssl", "x509", "-inform", "DER", data=der))
    intermediate_subject = run("openssl", "x509", "-in", str(intermediate), "-noout", "-subject", "-nameopt", "RFC2253").decode().strip().split("=", 1)[1]
    if issuer != intermediate_subject:
        raise SystemExit("Apple intermediate does not match the certificate issuer.")
    run("openssl", "verify", "-ignore_critical", "-partial_chain", "-trusted", str(intermediate), str(cert))
    password = secrets.token_hex(32)
    bundle = directory / "identity.p12"
    environment = os.environ.copy()
    environment["DBX_P12_EXPORT_PASSWORD"] = password
    run("openssl", "pkcs12", "-export", "-inkey", str(key), "-in", str(cert),
        "-certfile", str(intermediate), "-out", str(bundle),
        "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1",
        "-passout", "env:DBX_P12_EXPORT_PASSWORD", env=environment)
    # Owner-only backup password; never print it or put it into repository files.
    (directory / "identity-password.txt").write_text(password + "\n")
    values = {
        "APPLE_CERTIFICATE_BASE64": base64.b64encode(bundle.read_bytes()),
        "APPLE_CERTIFICATE_PASSWORD": password.encode(),
        "APPLE_TEAM_ID": args.team.encode(),
    }
    for name, value in values.items():
        run("gh", "secret", "set", name, "--repo", args.repo, data=value)
        print("Configured GitHub secret:", name)
    print("Certificate, private-key match, and issuer signature verified.")
    print("Owner-only signing backup:", directory)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError:
        # Suppress subprocess diagnostics that could contain sensitive arguments.
        raise SystemExit("Certificate validation or secret installation failed; no secret values were logged.")
