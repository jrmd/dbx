"""Seed only missing DBX QA files; retain the test vault and edits on relaunch."""
import json
from pathlib import Path
import sqlite3
import sys
import uuid

qa_dir = Path(sys.argv[1]).resolve()
database = qa_dir / "workbench.sqlite"
if not database.exists():
    with sqlite3.connect(database) as connection:
        connection.executescript("""
            PRAGMA foreign_keys = ON;
            CREATE TABLE teams (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE projects (
                id INTEGER PRIMARY KEY,
                team_id INTEGER NOT NULL REFERENCES teams(id),
                name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                budget NUMERIC,
                notes TEXT
            );
            INSERT INTO teams VALUES (1, 'Platform'), (2, 'Product'), (3, 'Support');
        """)
        connection.executemany(
            "INSERT INTO projects VALUES (?, ?, ?, ?, ?, ?)",
            [(i, (i % 3) + 1, f"Project {i:03d}",
              "paused" if i % 7 == 0 else "active", i * 125.5,
              None if i % 4 == 0 else "Fixture row for native workbench QA")
             for i in range(1, 251)],
        )

profiles = qa_dir / "config/dbx/connections.json"
if not profiles.exists():
    entries = [
        ("QA SQLite", "sqlite", f"sqlite://{database}"),
        ("QA PostgreSQL", "postgresql", "postgres://dbx_test@127.0.0.1:55432/dbx_test"),
        ("QA MySQL", "mysql", "mysql://dbx_test@127.0.0.1:53306/dbx_test"),
        ("QA Redis", "redis", "redis://127.0.0.1:56379/0"),
    ]
    profiles.write_text(json.dumps({"version": 1, "connections": [
        {"id": str(uuid.uuid4()), "name": name, "kind": kind, "url": url,
         "environment": "local", "max_connections": 5, "connect_timeout_ms": 5000,
         **({"secret_key": f"qa-{kind}"} if kind in ("postgresql", "mysql") else {})}
        for name, kind, url in entries
    ]}, indent=2) + "\n")
