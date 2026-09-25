#!/usr/bin/env bash
# Linux desktop QA with a persistent, isolated vault and disposable databases.
set -Eeuo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_root=$(cd -- "$script_dir/.." && pwd)
if [[ $(uname -s) != Linux ]]; then
  echo "This launcher uses Linux XDG config isolation." >&2
  exit 1
fi
qa_dir="$project_root/target/ui-test"
umask 077
mkdir -p "$qa_dir/config/dbx" "$qa_dir/artifacts"
python3 "$script_dir/seed-ui-test.py" "$qa_dir"
cd "$project_root"
cargo run --quiet --locked -p dbx-ui --example seed_qa_vault
if [[ ${DBX_UI_TEST_SKIP_BUILD:-0} != 1 ]]; then
  cargo build --locked -p dbx-ui
fi
echo "QA state: $qa_dir"
echo "Use a disposable vault passphrase; never save real connections here."
cd "$qa_dir"
exec env XDG_CONFIG_HOME="$qa_dir/config" "$project_root/target/debug/dbx" "$@"
