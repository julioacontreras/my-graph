"""Exceptions raised by the client."""

from __future__ import annotations


class MyGraphError(Exception):
    """Base class for every error raised by this library."""


class ProtocolError(MyGraphError):
    """The server sent something that does not follow protocol v1, or
    rejected a malformed request."""


class ConnectionClosedError(MyGraphError):
    """The connection is closed or was lost."""


class QueryError(MyGraphError):
    """The server could not run a statement. ``code`` is the engine error
    kind (``Syntax``, ``NotFound``, ``Transaction``, ``Type``, ...)."""

    def __init__(self, code: str, message: str, line: int | None = None, column: int | None = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.line = line
        self.column = column


class CypherSyntaxError(QueryError):
    """Invalid Cypher. ``line`` and ``column`` point at the problem."""


class NotFoundError(QueryError):
    """A database, schema or other named object does not exist."""


class AlreadyExistsError(QueryError):
    """A database or schema with that name already exists."""


class ConstraintError(QueryError):
    """A write broke a unique or not-null constraint. The statement was
    undone; earlier statements of an open transaction are kept."""


class TransactionError(QueryError):
    """Invalid transaction use, such as COMMIT with no open transaction."""


_BY_CODE: dict[str, type[QueryError]] = {
    "Syntax": CypherSyntaxError,
    "NotFound": NotFoundError,
    "AlreadyExists": AlreadyExistsError,
    "ConstraintViolation": ConstraintError,
    "Transaction": TransactionError,
}


def error_from_payload(payload: dict) -> MyGraphError:
    """Builds the exception for an ERROR message payload."""
    code = str(payload.get("code", "Unknown"))
    message = str(payload.get("message", "unknown error"))
    if code == "Protocol":
        return ProtocolError(message)
    cls = _BY_CODE.get(code, QueryError)
    return cls(code, message, payload.get("line"), payload.get("column"))
