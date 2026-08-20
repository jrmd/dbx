#!/usr/bin/env bash

set -Eeuo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_root=$(cd -- "$script_dir/.." && pwd)
compose_file="$project_root/docker-compose.test.yml"
compose=(docker compose -f "$compose_file" -p dbx-integration-test)
sqlite_dir=$(mktemp -d "${TMPDIR:-/tmp}/dbx-integration.XXXXXX")

cleanup() {
  status=$?
  "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
  rm -rf -- "$sqlite_dir"
  exit "$status"
}
trap cleanup EXIT

if ! command -v docker >/dev/null 2>&1; then
  echo "Docker is required for DBX integration tests" >&2
  exit 1
fi

"${compose[@]}" config --quiet
"${compose[@]}" up -d --wait

: "${DBX_TEST_POSTGRES_URL:=postgres://dbx_test:dbx_test_password@127.0.0.1:55432/dbx_test}"
: "${DBX_TEST_MYSQL_URL:=mysql://dbx_test:dbx_test_password@127.0.0.1:53306/dbx_test}"
: "${DBX_TEST_REDIS_URL:=redis://127.0.0.1:56379/0}"
: "${DBX_TEST_SQLITE_URL:=sqlite://$sqlite_dir/dbx.sqlite?mode=rwc}"
export DBX_TEST_POSTGRES_URL DBX_TEST_MYSQL_URL DBX_TEST_REDIS_URL DBX_TEST_SQLITE_URL

cargo test -p dbx-core --test integration -- --ignored --test-threads=1
