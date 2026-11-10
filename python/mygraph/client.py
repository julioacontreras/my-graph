"""Connection to a MyGraph server."""

from __future__ import annotations

import socket
import threading
from typing import Any, Mapping

from . import _protocol as p
from ._ops import CypherOps, quote
from .errors import ConnectionClosedError, ProtocolError, TransactionError, error_from_payload
from .transaction import Transaction
from .types import Result, Stats

DEFAULT_PORT = 7687


class Client(CypherOps):
    """One TCP connection to a MyGraph server.

    Requests on one client are sent one at a time (a lock serializes them),
    so a client can be shared between threads, but they all share its current
    database and its open transaction. Use one client per thread for
    independent work.
    """

    def __init__(
        self,
        host: str = "localhost",
        port: int = DEFAULT_PORT,
        *,
        user: str | None = None,
        password: str | None = None,
        timeout: float | None = 30.0,
        client_name: str = "mygraph-python/0.1.0",
    ):
        self.host, self.port = host, port
        self._lock = threading.Lock()
        self._next_id = 1
        self._transaction: Transaction | None = None
        try:
            self._sock: socket.socket | None = socket.create_connection((host, port), timeout=timeout)
        except OSError as e:
            raise ConnectionClosedError(f"cannot connect to {host}:{port}: {e}") from e
        self._sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        try:
            hello = self._request(p.HELLO, {"client": client_name}, expect=p.HELLO)
            self.server_info: dict[str, Any] = hello
            if user is not None:
                self._request(p.AUTH, {"user": user, "password": password or ""})
        except BaseException:
            self._close_socket()
            raise

    # ------------------------------------------------------------ transport

    def _request(self, message_type: int, payload: dict, expect: int = p.OK) -> dict:
        with self._lock:
            if self._sock is None:
                raise ConnectionClosedError("the client is closed")
            request_id = self._next_id
            self._next_id = (self._next_id + 1) & 0xFFFFFFFF or 1
            try:
                self._sock.sendall(p.encode_frame(message_type, request_id, p.encode_payload(payload)))
                kind, response_id, body = p.read_frame(self._sock)
            except (OSError, ConnectionClosedError) as e:
                # The stream position is unknown now; the connection cannot be reused.
                self._close_socket()
                if isinstance(e, ConnectionClosedError):
                    raise
                raise ConnectionClosedError(f"connection lost: {e}") from e
            except ProtocolError:
                self._close_socket()
                raise
        if response_id != request_id:
            self.close()
            raise ProtocolError(f"response id {response_id} does not match request id {request_id}")
        data = p.decode_payload(body)
        if kind == p.ERROR:
            raise error_from_payload(data)
        if kind != expect:
            raise ProtocolError(f"unexpected message type 0x{kind:02x} (expected 0x{expect:02x})")
        return data

    def _close_socket(self) -> None:
        sock, self._sock = self._sock, None
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass

    # ------------------------------------------------------------ queries

    def run(self, query: str, params: Mapping[str, Any] | None = None, **kwargs: Any) -> Result:
        """Runs a Cypher script (one or more ``;``-separated statements) and
        returns the last statement's rows. Parameters are referenced as
        ``$name`` and may be given as a dict, keyword arguments, or both."""
        all_params = {**(params or {}), **kwargs}
        data = self._request(p.QUERY, {"query": query, "params": all_params}, expect=p.QUERY_RESULT)
        return Result(
            columns=list(data.get("columns", [])),
            rows=list(data.get("rows", [])),
            stats=Stats.from_map(data.get("stats", {})),
            database=data.get("database", ""),
            schema=data.get("schema", ""),
            in_transaction=bool(data.get("in_transaction", False)),
        )

    def transaction(self) -> Transaction:
        """Begins an explicit transaction (``BEGIN``)."""
        if self._transaction is not None and self._transaction.is_open:
            raise TransactionError("Transaction", "a transaction is already open on this client")
        self._request(p.BEGIN, {})
        self._transaction = Transaction(self)
        return self._transaction

    begin = transaction

    def ping(self) -> None:
        self._request(p.PING, {}, expect=p.PONG)

    # ------------------------------------------------------------ databases and schemas

    def use_database(self, name: str) -> None:
        self.run(f"USE DATABASE {quote(name)}")

    def use_schema(self, name: str) -> None:
        self.run(f"USE SCHEMA {quote(name)}")

    def create_database(self, name: str, *, if_not_exists: bool = True) -> None:
        self.run(f"CREATE DATABASE {'IF NOT EXISTS ' if if_not_exists else ''}{quote(name)}")

    def drop_database(self, name: str, *, if_exists: bool = True) -> None:
        self.run(f"DROP DATABASE {'IF EXISTS ' if if_exists else ''}{quote(name)}")

    def create_schema(self, name: str, *, if_not_exists: bool = True) -> None:
        self.run(f"CREATE SCHEMA {'IF NOT EXISTS ' if if_not_exists else ''}{quote(name)}")

    def drop_schema(self, name: str, *, if_exists: bool = True) -> None:
        self.run(f"DROP SCHEMA {'IF EXISTS ' if if_exists else ''}{quote(name)}")

    def databases(self) -> list[dict[str, Any]]:
        return self.run("SHOW DATABASES").records()

    def schemas(self) -> list[dict[str, Any]]:
        return self.run("SHOW SCHEMAS").records()

    # ------------------------------------------------------------ lifecycle

    @property
    def closed(self) -> bool:
        return self._sock is None

    def close(self) -> None:
        """Says GOODBYE and closes the socket. An open transaction is rolled
        back by the server."""
        if self._sock is None:
            return
        try:
            self._request(p.GOODBYE, {})
        except Exception:
            pass
        self._close_socket()

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    def __repr__(self) -> str:
        state = "closed" if self.closed else "open"
        return f"<mygraph.Client {self.host}:{self.port} {state}>"


def connect(host: str = "localhost", port: int = DEFAULT_PORT, **kwargs: Any) -> Client:
    """Opens a connection. Same arguments as :class:`Client`."""
    return Client(host, port, **kwargs)
