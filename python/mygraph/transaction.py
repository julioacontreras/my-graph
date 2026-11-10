"""Explicit transactions."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Mapping

from . import _protocol as p
from ._ops import CypherOps
from .errors import TransactionError
from .types import Result

if TYPE_CHECKING:
    from .client import Client


class Transaction(CypherOps):
    """A transaction on one connection. Use it as a context manager:

        with db.transaction() as tx:
            tx.create_node("User", {"name": "Carol"})

    Leaving the block commits; an exception rolls back. While it is open,
    every statement on the same client runs inside it. In MyGraph v0.1 an open
    transaction blocks all other connections, so keep it short.
    """

    def __init__(self, client: "Client"):
        self._client = client
        self._open = True

    @property
    def is_open(self) -> bool:
        return self._open

    def run(self, query: str, params: Mapping[str, Any] | None = None, **kwargs: Any) -> Result:
        self._check()
        return self._client.run(query, params, **kwargs)

    def commit(self) -> None:
        self._finish(p.COMMIT)

    def rollback(self) -> None:
        self._finish(p.ROLLBACK)

    def _check(self) -> None:
        if not self._open:
            raise TransactionError("Transaction", "the transaction is already finished")

    def _finish(self, message_type: int) -> None:
        self._check()
        self._open = False
        self._client._request(message_type, {})
        self._client._transaction = None

    def __enter__(self) -> "Transaction":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        if not self._open:
            return
        if exc_type is None:
            self.commit()
        else:
            try:
                self.rollback()
            except Exception:
                pass  # keep the original exception
