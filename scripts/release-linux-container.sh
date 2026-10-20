#!/usr/bin/env bash
set -Eeuo pipefail
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || {
  echo 'Release container must run as linux/amd64.' >&2; exit 1;
}
cargo fmt --all -- --check
cargo test --locked --release --workspace
cargo clippy --locked --release --workspace --all-targets -- -D warnings
bash scripts/build-linux-app.sh appimage
