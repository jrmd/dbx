#!/usr/bin/env python3
"""DBX protocol v1 example: a SQLite adapter using only Python's stdlib.

Register the accompanying manifest after replacing its absolute paths. This
example illustrates query/browse/bound parameters; it is not an alternate
transaction engine. Each invocation owns one short-lived SQLite connection.
"""
import json
from pathlib import Path
import sqlite3
import sys
from urllib.parse import unquote, urlparse


def quote(identifier):
    return '"' + identifier.replace('"', '""') + '"'


def cell(value):
    if value is None:
        return {"type": "null"}
    if isinstance(value, int):
        return {"type": "integer", "value": value}
    if isinstance(value, float):
        return {"type": "real", "value": value}
    if isinstance(value, bytes):
        return {"type": "bytes", "value": list(value)}
    return {"type": "text", "value": value}


def parameter(value):
    kind, data = value["type"], value.get("value")
    if kind == "null":
        return None
    if kind == "bytes":
        return bytes(data)
    if kind == "json":
        return json.dumps(data, separators=(",", ":"))
    return data


def run(request):
    if request["protocol"] != 1:
        raise ValueError("Unsupported protocol")
    config, args = request["connection"], request["parameters"]
    address = urlparse(config["url"])
    if address.scheme != "example-sqlite" or address.netloc not in ("", "localhost"):
        raise ValueError("Expected example-sqlite:///absolute/database/path")
    path = Path(unquote(address.path)).resolve()
    uri = path.as_uri() + ("?mode=ro" if config["read_only"] else "?mode=rwc")
    db = sqlite3.connect(uri, uri=True)
    operation = request["operation"]
    if operation == "connect":
        return str(path)
    if operation == "list_databases":
        return [str(path)]
    if operation == "use_database":
        raise ValueError("Open another connection to switch files")
    if operation == "list_tables":
        return [{"name": row[0], "schema": None, "kind": row[1]}
                for row in db.execute("SELECT name,type FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%'")]
    if operation in ("describe_table", "table_structure"):
        name = args["table"]["name"]
        columns = [{"name": row[1], "data_type": row[2] or "TEXT", "nullable": not bool(row[3]),
                    "ordinal": row[0], "primary_key": bool(row[5]), "enum_values": [], "default_value": row[4]}
                   for row in db.execute("PRAGMA table_info(" + quote(name) + ")")]
        if operation == "describe_table":
            return columns
        definition = db.execute("SELECT sql FROM sqlite_master WHERE name=?", (name,)).fetchone()
        return {"columns": columns, "foreign_keys": [], "indexes": [], "checks": [],
                "definition": definition[0] if definition else None}
    if operation in ("query", "query_statement", "execute"):
        statement = args.get("statement", {"sql": args.get("query"), "params": []})
        cursor = db.execute(statement["sql"], [parameter(value) for value in statement["params"]])
        if operation == "execute":
            db.commit()
            return {"rows_affected": max(0, cursor.rowcount), "last_insert_id": cursor.lastrowid, "elapsed_ms": 0}
        columns = [{"name": item[0], "data_type": "VALUE", "nullable": True, "ordinal": index,
                    "primary_key": False, "enum_values": [], "default_value": None}
                   for index, item in enumerate(cursor.description or [])]
        limit = args.get("options", {}).get("max_rows")
        limit = min(limit if limit is not None else 100000, 100000)
        rows = cursor.fetchmany(limit + 1) if cursor.description else []
        db.commit()
        return {"columns": columns, "rows": [{"values": [cell(value) for value in row]} for row in rows[:limit]],
                "rows_affected": None if cursor.description else max(0, cursor.rowcount),
                "truncated": len(rows) > limit, "elapsed_ms": 0}
    raise ValueError("Unsupported operation")


if __name__ == "__main__":
    try:
        result = run(json.load(sys.stdin))
        response = {"protocol": 1, "ok": True, "result": result}
    except (ValueError, KeyError, TypeError, OSError, sqlite3.Error):
        response = {"protocol": 1, "ok": False}
    json.dump(response, sys.stdout, separators=(",", ":"))
