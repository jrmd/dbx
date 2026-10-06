#!/usr/bin/env bash

set -Eeuo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_root=$(cd -- "$script_dir/.." && pwd)
compose_file="$project_root/docker-compose.test.yml"
compose=(docker compose -f "$compose_file" -p "${DBX_TEST_COMPOSE_PROJECT:-dbx-integration-test}")
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
# Build before booting the services so native dependency compilation does not
# compete with SQL Server, Kafka and Elasticsearch for runner memory.
cargo test --locked -p dbx-core --tests --no-run
"${compose[@]}" up -d --wait

: "${DBX_TEST_POSTGRES_URL:=postgres://dbx_test:dbx_test_password@127.0.0.1:${DBX_TEST_POSTGRES_PORT:-55432}/dbx_test}"
: "${DBX_TEST_MYSQL_URL:=mysql://dbx_test:dbx_test_password@127.0.0.1:${DBX_TEST_MYSQL_PORT:-53306}/dbx_test}"
: "${DBX_TEST_REDIS_URL:=redis://127.0.0.1:${DBX_TEST_REDIS_PORT:-56379}/0}"
: "${DBX_TEST_SQLITE_URL:=sqlite://$sqlite_dir/dbx.sqlite?mode=rwc}"
: "${DBX_TEST_CLICKHOUSE_URL:=clickhouse://dbx_test:dbx_test_password@127.0.0.1:${DBX_TEST_CLICKHOUSE_PORT:-58123}/dbx_test}"
: "${DBX_TEST_MONGO_URL:=mongodb://localhost:${DBX_TEST_MONGO_PORT:-57017}/dbx_qa}"
: "${DBX_TEST_COCKROACH_URL:=postgres://root@localhost:${DBX_TEST_COCKROACH_PORT:-56257}/defaultdb?sslmode=disable}"
: "${DBX_TEST_ELASTICSEARCH_URL:=http://localhost:${DBX_TEST_ELASTICSEARCH_PORT:-59200}}"
: "${DBX_TEST_KAFKA_URL:=kafka://localhost:${DBX_TEST_KAFKA_PORT:-59092}}"
: "${DBX_TEST_SQLSERVER_URL:=sqlserver://sa:Dbx_test_Passw0rd@127.0.0.1:${DBX_TEST_SQLSERVER_PORT:-51433}/master?trust_server_certificate=true}"
export DBX_TEST_CLICKHOUSE_URL DBX_TEST_MONGO_URL DBX_TEST_COCKROACH_URL DBX_TEST_ELASTICSEARCH_URL DBX_TEST_KAFKA_URL DBX_TEST_SQLSERVER_URL
export DBX_TEST_POSTGRES_URL DBX_TEST_MYSQL_URL DBX_TEST_REDIS_URL DBX_TEST_SQLITE_URL

cargo test --locked -p dbx-core --test integration -- --ignored --test-threads=1 --skip socket_and_ssh_connections_integration
cargo test --locked -p dbx-core --test workbench_safety -- --ignored --test-threads=1
cargo test --locked -p dbx-core --test connectors -- --ignored --test-threads=1
