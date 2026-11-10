"""``mygraph-client`` command: run Cypher against a MyGraph server.

    mygraph-client [--host HOST] [--port PORT] [--json] "MATCH (n) RETURN n;"
    echo "MATCH (n) RETURN n;" | mygraph-client

Without a query argument the script is read from standard input.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
from typing import Any

from .client import DEFAULT_PORT, connect
from .errors import MyGraphError
from .types import Node, Path, Relationship


def _plain(value: Any) -> Any:
    if isinstance(value, (Node, Relationship, Path)):
        return dataclasses.asdict(value)
    if isinstance(value, list):
        return [_plain(v) for v in value]
    if isinstance(value, dict):
        return {k: _plain(v) for k, v in value.items()}
    return value


def _show(value: Any) -> str:
    if isinstance(value, Node):
        labels = "".join(f":{label}" for label in value.labels)
        return f"({value.id}{labels} {json.dumps(value.properties, ensure_ascii=False)})"
    if isinstance(value, Relationship):
        props = json.dumps(value.properties, ensure_ascii=False)
        return f"({value.start_node})-[{value.id}:{value.type} {props}]->({value.end_node})"
    if isinstance(value, Path):
        return " ".join(_show(x) for pair in zip(value.nodes, value.relationships) for x in pair) + (
            f" {_show(value.end)}" if value.nodes else ""
        )
    if isinstance(value, str):
        return value
    return json.dumps(_plain(value), ensure_ascii=False)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="mygraph-client", description="Run Cypher against a MyGraph server."
    )
    parser.add_argument("query", nargs="?", help="Cypher script (default: read stdin)")
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--json", action="store_true", help="print records as JSON lines")
    args = parser.parse_args(argv)

    query = args.query if args.query is not None else sys.stdin.read()
    if not query.strip():
        parser.error("empty query")

    try:
        with connect(args.host, args.port) as db:
            result = db.run(query)
    except MyGraphError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    if args.json:
        for record in result:
            print(json.dumps(_plain(record), ensure_ascii=False))
    else:
        if result.columns:
            print("\t".join(result.columns))
        for row in result.rows:
            print("\t".join(_show(v) for v in row))
        print(f"({len(result)} rows)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
