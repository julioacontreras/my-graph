"""Python client for the MyGraph server.

    from mygraph import connect

    with connect("localhost", 6660) as db:
        alice = db.create_node("User", {"name": "Alice"})
        for user in db.find_nodes("User", {"name": "Alice"}):
            print(user.id, user.properties)

The client speaks MyGraph protocol v1 (``PROTOCOL.md``) over TCP and
needs nothing outside the Python standard library.
"""

from .client import Client, connect
from .errors import (
    AlreadyExistsError,
    ConnectionClosedError,
    ConstraintError,
    CypherSyntaxError,
    MyGraphError,
    NotFoundError,
    ProtocolError,
    QueryError,
    TransactionError,
)
from ._version import __version__
from .transaction import Transaction
from .types import Node, Path, Relationship, Result, Stats


__all__ = [
    "AlreadyExistsError",
    "Client",
    "ConnectionClosedError",
    "ConstraintError",
    "CypherSyntaxError",
    "MyGraphError",
    "Node",
    "NotFoundError",
    "Path",
    "ProtocolError",
    "QueryError",
    "Relationship",
    "Result",
    "Stats",
    "Transaction",
    "TransactionError",
    "connect",
]
