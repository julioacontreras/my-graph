"""Search, create, update and delete against a running server.

    cargo run -p mygraph-server -- start
    python3 python/examples/crud.py
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))  # run from a checkout without installing

import mygraph

with mygraph.connect("localhost", 7687) as db:
    print("connected to", db.server_info["server"], db.server_info["version"])

    # Create
    alice = db.create_node("User", {"name": "Alice", "age": 25})
    bob = db.create_node("User", {"name": "Bob", "age": 30})
    knows = db.create_relationship(alice.id, bob.id, "KNOWS", {"since": 2020})

    # Search
    for user in db.find_nodes("User", order_by="age"):
        print("user:", user.id, user.properties)
    r = db.run("MATCH (a:User)-[k:KNOWS]->(b:User) RETURN a.name AS a, b.name AS b, k.since AS since")
    for record in r:
        print(f"{record['a']} knows {record['b']} since {record['since']}")

    # Update
    print("updated:", db.update_node(alice.id, {"age": 26}).properties)

    # Delete, in one transaction
    with db.transaction() as tx:
        tx.delete_relationship(knows.id)
        tx.delete_nodes("User")
    print("users left:", db.count_nodes("User"))
