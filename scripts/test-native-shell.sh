#!/usr/bin/env bash
set -euo pipefail
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'Native shell lifecycle: skipped (macOS only)'
  exit 0
fi
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
qa_binary="$(mktemp -d "${TMPDIR:-/tmp}/dbx-native-shell.XXXXXX")"
trap 'rm -rf "$qa_binary"' EXIT
xcrun swiftc -parse-as-library \
  "$repo_root/crates/dbx-ui/native/NativeShell.swift" \
  "$repo_root/crates/dbx-ui/native/NativeShellTests.swift" \
  -o "$qa_binary/native-shell-tests"
"$qa_binary/native-shell-tests"
