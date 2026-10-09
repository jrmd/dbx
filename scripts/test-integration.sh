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

cargo_profile=()
if [[ ${DBX_TEST_CARGO_PROFILE:-debug} == release ]]; then cargo_profile=(--release); fi

"${compose[@]}" config --quiet
# Build before booting the services so native dependency compilation does not
# compete with SQL Server, Kafka and Elasticsearch for runner memory.
cargo test --locked "${cargo_profile[@]}" -p dbx-core --tests --no-run
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

# Run the exact server-version native clients in disposable containers. Neither
# developers nor runners need to replace their system database clients.
if [[ ${DBX_TEST_NATIVE_CLIENTS:-containers} == containers ]]; then
  native_client() {
    local directory=$1 client=$2 image=$3 entrypoint=${4:-$2}
    mkdir -p "$directory"
    cat > "$directory/$client" <<EOF
#!/bin/sh
exec docker run --rm -i --network host --user "\$(id -u):\$(id -g)" -v /tmp:/tmp:ro -v "\$PWD:\$PWD:ro" -w "\$PWD" -e PGHOST -e PGHOSTADDR -e PGPORT -e PGDATABASE -e PGUSER -e PGPASSFILE -e PGCONNECT_TIMEOUT -e PGSSLMODE -e PGSSLROOTCERT -e PGSSLCERT -e PGSSLKEY --entrypoint $entrypoint $image "\$@"
EOF
    chmod 700 "$directory/$client"
  }
  for client in pg_dump pg_restore; do native_client "$sqlite_dir/bin" "$client" postgres:16-alpine; done
  for client in mysqldump mysql; do native_client "$sqlite_dir/bin" "$client" mysql:8.4; done
  # MariaDB clients reject several MySQL client options. Runners may have a
  # MySQL client installed, so the MariaDB pass shadows the MySQL names too.
  native_client "$sqlite_dir/mariadb-bin" mysqldump mariadb:11.4 mariadb-dump
  native_client "$sqlite_dir/mariadb-bin" mysql mariadb:11.4 mariadb
  mariadb_path="$sqlite_dir/mariadb-bin:$PATH"
  export PATH="$sqlite_dir/bin:$PATH"
fi
: "${DBX_TEST_MYSQL_ADMIN_URL:=mysql://root:dbx_test_root_password@127.0.0.1:${DBX_TEST_MYSQL_PORT:-53306}/dbx_test}"
export DBX_TEST_MYSQL_ADMIN_URL

cargo test --locked "${cargo_profile[@]}" -p dbx-core --test integration -- --ignored --test-threads=1 --skip socket_and_ssh_connections_integration --skip strict_tls_over_ssh_integration --skip native_postgres_backup_over_password_socket
cargo test --locked "${cargo_profile[@]}" -p dbx-core --test workbench_safety -- --ignored --test-threads=1
cargo test --locked "${cargo_profile[@]}" -p dbx-core --test connectors -- --ignored --test-threads=1

# MariaDB speaks the MySQL protocol, so the MySQL tests run against it too.
mariadb_port=${DBX_TEST_MARIADB_PORT:-53307}
mariadb_tests=(env PATH="${mariadb_path:-$PATH}"
  DBX_TEST_MYSQL_URL="${DBX_TEST_MARIADB_URL:-mysql://dbx_test:dbx_test_password@127.0.0.1:$mariadb_port/dbx_test}"
  DBX_TEST_MYSQL_ADMIN_URL="${DBX_TEST_MARIADB_ADMIN_URL:-mysql://root:dbx_test_root_password@127.0.0.1:$mariadb_port/dbx_test}"
  cargo test --locked "${cargo_profile[@]}" -p dbx-core)
"${mariadb_tests[@]}" --test integration mysql -- --ignored --test-threads=1
"${mariadb_tests[@]}" --test workbench_safety mysql -- --ignored --test-threads=1
