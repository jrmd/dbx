#!/usr/bin/env python3
"""Live local socket/SSH checks. All keys, databases and known-hosts are disposable."""
import os
from pathlib import Path
import pwd
import shlex
import shutil
import socket
import subprocess
import tempfile
import time


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def main():
    repo = Path(__file__).resolve().parent.parent
    names = []
    daemon = None
    with tempfile.TemporaryDirectory(prefix="dbx-transports-") as temporary:
        root = Path(temporary)
        sockets = root / "sockets"
        sockets.mkdir(mode=0o777)
        sockets.chmod(0o777)
        (sockets / "pg").mkdir(mode=0o777)
        (sockets / "pg").chmod(0o777)
        try:
            for key in ("identity", "host"):
                run("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / key))
            (root / "authorized").write_text((root / "identity.pub").read_text())
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                ssh_port = listener.getsockname()[1]
            username = pwd.getpwuid(os.getuid()).pw_name
            host_key = (root / "host.pub").read_text().split()
            (root / "known_hosts").write_text(f"[127.0.0.1]:{ssh_port} {host_key[0]} {host_key[1]}\n")
            (root / "sshd_config").write_text(
                f"Port {ssh_port}\nListenAddress 127.0.0.1\nHostKey {root}/host\n"
                f"AuthorizedKeysFile {root}/authorized\nPidFile {root}/sshd.pid\n"
                f"AllowUsers {username}\nStrictModes no\nUsePAM no\n"
                "PasswordAuthentication no\nKbdInteractiveAuthentication no\n"
                "AllowTcpForwarding yes\nAllowStreamLocalForwarding yes\n")
            (root / "ssh_config").write_text(
                f"Host *\n  UserKnownHostsFile {root}/known_hosts\n"
                "  GlobalKnownHostsFile /dev/null\n  LogLevel ERROR\n")
            client = shutil.which("ssh")
            (root / "bin").mkdir()
            wrapper = root / "bin/ssh"
            wrapper.write_text(f"#!/bin/sh\nexec {shlex.quote(client)} -F {shlex.quote(str(root / 'ssh_config'))} \"$@\"\n")
            wrapper.chmod(0o700)
            log = open(root / "sshd.log", "w+")
            daemon = subprocess.Popen([shutil.which("sshd"), "-D", "-e", "-f", str(root / "sshd_config")], stderr=log)
            for _ in range(50):
                if daemon.poll() is not None:
                    log.seek(0)
                    raise RuntimeError(log.read())
                try:
                    with socket.create_connection(("127.0.0.1", ssh_port), timeout=0.2):
                        break
                except OSError:
                    time.sleep(0.1)
            services = [
                ("postgres", "postgres:16-alpine", "55432:5432", ["-e", "POSTGRES_DB=dbx_test", "-e", "POSTGRES_USER=dbx_test", "-e", "POSTGRES_PASSWORD=dbx_test_password"], ["postgres", "-c", "unix_socket_directories=/var/run/postgresql,/sockets/pg"]),
                ("mysql", "mysql:8.4", "53306:3306", ["-e", "MYSQL_DATABASE=dbx_test", "-e", "MYSQL_USER=dbx_test", "-e", "MYSQL_PASSWORD=dbx_test_password", "-e", "MYSQL_ROOT_PASSWORD=dbx_test_root_password"], ["--socket=/sockets/mysql.sock"]),
                ("redis", "redis:7-alpine", "56379:6379", [], ["redis-server", "--unixsocket", "/sockets/redis.sock", "--unixsocketperm", "777"]),
            ]
            for name, image, port, environment, command in services:
                container = f"dbx-transports-{os.getpid()}-{name}"
                names.append(container)
                container_port = port.split(":")[1]
                ssh_binding = ["-p", "127.0.0.1::2222"] if name == "redis" else []
                run("docker", "run", "-d", "--name", container, "-p", f"127.0.0.1::{container_port}", "-v", f"{sockets}:/sockets", "-v", f"{sockets}:{sockets}", *ssh_binding, *environment, image, *command, stdout=subprocess.DEVNULL)
            for _ in range(120):
                for name in names:
                    state = subprocess.run(["docker", "inspect", "-f", "{{.State.Running}}", name], capture_output=True, text=True, check=True)
                    if state.stdout.strip() != "true":
                        details = subprocess.run(["docker", "logs", name], capture_output=True, text=True)
                        raise RuntimeError(details.stdout + details.stderr)
                checks = [
                    ["docker", "exec", names[0], "pg_isready", "-U", "dbx_test", "-d", "dbx_test"],
                    ["docker", "exec", names[1], "mysqladmin", "ping", "-h", "127.0.0.1", "--silent"],
                    ["docker", "exec", names[2], "redis-cli", "ping"],
                ]
                if all(subprocess.run(check, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0 for check in checks):
                    break
                time.sleep(1)
            else:
                raise RuntimeError("Disposable databases did not become ready")
            # TLS identity belongs to the original server, even when the byte
            # transport runs through a local SSH socket. A second CA is untrusted.
            for stem in ("ca", "untrusted"):
                run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", f"/CN=DBX fixture {stem}", "-addext", "basicConstraints=critical,CA:TRUE", "-keyout", str(root / f"{stem}.key"), "-out", str(root / f"{stem}.crt"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            run("openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", str(root / "server.key"), "-out", str(root / "server.csr"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            (root / "server.ext").write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost\n")
            run("openssl", "x509", "-req", "-in", str(root / "server.csr"), "-CA", str(root / "ca.crt"), "-CAkey", str(root / "ca.key"), "-CAcreateserial", "-days", "1", "-extfile", str(root / "server.ext"), "-out", str(root / "server.crt"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for index, destination, owner in [(0, "/var/lib/postgresql/data", "postgres"), (1, "/var/lib/mysql", "mysql")]:
                run("docker", "cp", str(root / "server.crt"), f"{names[index]}:{destination}/server.crt", stdout=subprocess.DEVNULL)
                run("docker", "cp", str(root / "server.key"), f"{names[index]}:{destination}/server.key", stdout=subprocess.DEVNULL)
                run("docker", "exec", "-u", "root", names[index], "sh", "-c", f"chown {owner}:{owner} {destination}/server.* && chmod 600 {destination}/server.key")
            run("docker", "exec", names[0], "psql", "-U", "dbx_test", "-d", "dbx_test", "-c", "ALTER SYSTEM SET ssl = 'on'", stdout=subprocess.DEVNULL)
            run("docker", "exec", names[0], "psql", "-U", "dbx_test", "-d", "dbx_test", "-c", "SELECT pg_reload_conf()", stdout=subprocess.DEVNULL)
            run("docker", "exec", names[1], "sh", "-c", "cp /var/lib/mysql/server.crt /var/lib/mysql/server-cert.pem && cp /var/lib/mysql/server.key /var/lib/mysql/server-key.pem && chown mysql:mysql /var/lib/mysql/server-*.pem")
            run("docker", "restart", names[1], stdout=subprocess.DEVNULL)
            for _ in range(60):
                if subprocess.run(["docker", "exec", names[1], "mysqladmin", "ping", "-h", "127.0.0.1", "--silent"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0: break
                time.sleep(1)
            environment = os.environ | {
                "PATH": str(root / "bin") + os.pathsep + os.environ["PATH"],
                # Native socket jobs must not inherit a TCP destination override.
                "PGHOSTADDR": "192.0.2.1",
                "DBX_TEST_TRANSPORT_DIRECTORY": str(root),
                "DBX_TEST_SSH_PORT": str(ssh_port), "DBX_TEST_SSH_USER": username,
            }
            for index, (name, _, port, _, _) in enumerate(services):
                binding = subprocess.run(["docker", "port", names[index], port.split(":")[1]], capture_output=True, text=True, check=True)
                environment[f"DBX_TEST_TRANSPORT_{name.upper()}_PORT"] = binding.stdout.strip().rsplit(":", 1)[1]
            # Password authentication belongs to this disposable Redis container,
            # never to an account on the developer's machine.
            run("docker", "exec", names[2], "apk", "add", "--no-cache", "openssh", stdout=subprocess.DEVNULL)
            run("docker", "exec", names[2], "sh", "-c", "adduser -D dbx_password && printf '%s\\n' 'dbx_password:disposable-fixture-password' | chpasswd && ssh-keygen -A", stdout=subprocess.DEVNULL)
            password_config = "Port 2222\nListenAddress 0.0.0.0\nPasswordAuthentication yes\nPubkeyAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nAllowUsers dbx_password\nAllowTcpForwarding yes\n"
            run("docker", "exec", "-i", names[2], "sh", "-c", "cat > /tmp/dbx-sshd.conf", input=password_config, text=True)
            run("docker", "exec", "-d", names[2], "/usr/sbin/sshd", "-D", "-e", "-f", "/tmp/dbx-sshd.conf")
            binding = subprocess.run(["docker", "port", names[2], "2222"], capture_output=True, text=True, check=True)
            password_port = binding.stdout.strip().rsplit(":",1)[1]
            host_key = subprocess.run(["docker", "exec", names[2], "cat", "/etc/ssh/ssh_host_ed25519_key.pub"], capture_output=True, text=True, check=True).stdout.split()
            with (root / "known_hosts").open("a") as known_hosts:
                known_hosts.write(f"[127.0.0.1]:{password_port} {host_key[0]} {host_key[1]}\n")
            environment["DBX_TEST_SSH_PASSWORD_PORT"] = password_port
            # Real libpq tools use the exact same socket path inside the fixture.
            # Copy only the private job password file; never expose the host /tmp.
            native_wrapper = '''#!/usr/bin/env python3
import os, pathlib, subprocess, sys, uuid
container = CONTAINER
program = pathlib.Path(sys.argv[0]).name
credential = "/tmp/dbx-native-" + uuid.uuid4().hex
options = []
try:
    if "PGPASSFILE" in os.environ:
        subprocess.run(["docker", "exec", "-i", container, "sh", "-c", "umask 077; cat > " + credential], input=pathlib.Path(os.environ["PGPASSFILE"]).read_bytes(), check=True)
        options += ["--env", "PGPASSFILE=" + credential]
    for key in ["PGHOST", "PGHOSTADDR", "PGPORT", "PGDATABASE", "PGUSER", "PGCONNECT_TIMEOUT", "PGSSLMODE", "PGSSLROOTCERT", "PGSSLCERT", "PGSSLKEY"]:
        if key in os.environ:
            options += ["--env", key + "=" + os.environ[key]]
    arguments = list(sys.argv[1:])
    if program == "pg_restore" and arguments and not arguments[-1].startswith("-"):
        # Restore reads the archive from stdin rather than a host file path.
        with open(arguments.pop(), "rb") as archive:
            result = subprocess.run(["docker", "exec", "-i", *options, container, program, *arguments], stdin=archive)
    else:
        result = subprocess.run(["docker", "exec", *options, container, program, *arguments])
    sys.exit(result.returncode)
finally:
    subprocess.run(["docker", "exec", container, "rm", "-f", credential], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
'''.replace("CONTAINER", repr(names[0]))
            for program in ["pg_dump", "pg_restore"]:
                wrapper = root / "bin" / program
                wrapper.write_text(native_wrapper)
                wrapper.chmod(0o700)
            run("docker", "exec", names[0], "sh", "-c", "sed -i 's/^local[[:space:]].*/local all all scram-sha-256/' /var/lib/postgresql/data/pg_hba.conf")
            run("docker", "exec", names[0], "psql", "-U", "dbx_test", "-d", "dbx_test", "-c", "SELECT pg_reload_conf()", stdout=subprocess.DEVNULL)
            run("cargo", "test", "--locked", *(["--release"] if os.environ.get("DBX_TEST_CARGO_PROFILE") == "release" else []), "-p", "dbx-core", "socket_and_ssh_connections_integration", "--", "--ignored", "--nocapture", env=environment, cwd=repo)
            run("cargo", "test", "--locked", *(["--release"] if os.environ.get("DBX_TEST_CARGO_PROFILE") == "release" else []), "-p", "dbx-core", "native_postgres_backup_over_password_socket", "--", "--ignored", "--nocapture", env=environment, cwd=repo)
            run("cargo", "test", "--locked", *(["--release"] if os.environ.get("DBX_TEST_CARGO_PROFILE") == "release" else []), "-p", "dbx-core", "ssh_tunnel_lifetime_integration", "--", "--ignored", "--nocapture", env=environment, cwd=repo)
            run("cargo", "test", "--locked", *(["--release"] if os.environ.get("DBX_TEST_CARGO_PROFILE") == "release" else []), "-p", "dbx-core", "ssh_password_integration", "--", "--ignored", "--nocapture", env=environment, cwd=repo)
            run("cargo", "test", "--locked", *(["--release"] if os.environ.get("DBX_TEST_CARGO_PROFILE") == "release" else []), "-p", "dbx-core", "strict_tls_over_ssh_integration", "--", "--ignored", "--nocapture", env=environment, cwd=repo)
        finally:
            for name in names:
                subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if daemon:
                daemon.terminate()
                daemon.wait(timeout=5)


if __name__ == "__main__":
    main()
