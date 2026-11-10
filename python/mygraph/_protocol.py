"""MyGraph protocol v1: framing and value codec (see PROTOCOL.md).

Frame header, little-endian: u8 version, u8 message type, u32 request id,
u32 payload length. The payload is one encoded MAP (count + key/value pairs,
no tag).
"""

from __future__ import annotations

import socket
import struct
from typing import Any

from .errors import ConnectionClosedError, ProtocolError
from .types import Node, Path, Relationship

PROTOCOL_VERSION = 1
MAX_PAYLOAD = 64 * 1024 * 1024
MAX_DEPTH = 64

_HEADER = struct.Struct("<BBII")
_U32 = struct.Struct("<I")
_U64 = struct.Struct("<Q")
_I64 = struct.Struct("<q")
_F64 = struct.Struct("<d")

# Message types
HELLO = 0x01
AUTH = 0x02
QUERY = 0x10
QUERY_RESULT = 0x11
BEGIN = 0x20
COMMIT = 0x21
ROLLBACK = 0x22
OK = 0x23
PING = 0x30
PONG = 0x31
GOODBYE = 0x40
ERROR = 0x7F

# Value tags
T_NULL, T_BOOL, T_INT, T_FLOAT, T_STRING, T_BYTES, T_LIST, T_MAP, T_NODE, T_REL, T_PATH = range(11)

_I64_MIN, _I64_MAX = -(2**63), 2**63 - 1


# ---------------------------------------------------------------- encoding


def _len(buf: bytearray, n: int) -> None:
    if n > 0xFFFFFFFF:
        raise ValueError("value too large to encode")
    buf += _U32.pack(n)


def _bytes(buf: bytearray, b: bytes) -> None:
    _len(buf, len(b))
    buf += b


def encode_value(buf: bytearray, v: Any, depth: int = 0) -> None:
    if depth > MAX_DEPTH:
        raise ValueError("value nesting too deep")
    if v is None:
        buf.append(T_NULL)
    elif isinstance(v, bool):  # before int: bool is a subclass of int
        buf.append(T_BOOL)
        buf.append(1 if v else 0)
    elif isinstance(v, int):
        if not _I64_MIN <= v <= _I64_MAX:
            raise ValueError(f"integer {v} does not fit in 64 bits")
        buf.append(T_INT)
        buf += _I64.pack(v)
    elif isinstance(v, float):
        buf.append(T_FLOAT)
        buf += _F64.pack(v)
    elif isinstance(v, str):
        buf.append(T_STRING)
        _bytes(buf, v.encode("utf-8"))
    elif isinstance(v, (bytes, bytearray, memoryview)):
        buf.append(T_BYTES)
        _bytes(buf, bytes(v))
    elif isinstance(v, (list, tuple)):
        buf.append(T_LIST)
        _len(buf, len(v))
        for item in v:
            encode_value(buf, item, depth + 1)
    elif isinstance(v, dict):
        buf.append(T_MAP)
        encode_map(buf, v, depth + 1)
    elif isinstance(v, (Node, Relationship, Path)):
        raise TypeError(f"{type(v).__name__} values cannot be sent as parameters; pass its id instead")
    else:
        raise TypeError(f"cannot send a value of type {type(v).__name__}")


def encode_map(buf: bytearray, m: dict, depth: int = 0) -> None:
    _len(buf, len(m))
    for k, v in m.items():
        if not isinstance(k, str):
            raise TypeError(f"map keys must be str, got {type(k).__name__}")
        _bytes(buf, k.encode("utf-8"))
        encode_value(buf, v, depth)


def encode_payload(payload: dict) -> bytes:
    buf = bytearray()
    encode_map(buf, payload)
    return bytes(buf)


# ---------------------------------------------------------------- decoding


class Decoder:
    def __init__(self, data: bytes):
        self.data = memoryview(data)
        self.pos = 0

    def _take(self, n: int) -> memoryview:
        if len(self.data) - self.pos < n:
            raise ProtocolError("unexpected end of payload")
        s = self.data[self.pos : self.pos + n]
        self.pos += n
        return s

    def _u8(self) -> int:
        return self._take(1)[0]

    def _u32(self) -> int:
        return _U32.unpack(self._take(4))[0]

    def _u64(self) -> int:
        return _U64.unpack(self._take(8))[0]

    def _count(self) -> int:
        n = self._u32()
        if n > len(self.data) - self.pos:
            raise ProtocolError("element count exceeds payload size")
        return n

    def _str(self) -> str:
        n = self._u32()
        try:
            return str(self._take(n), "utf-8")
        except UnicodeDecodeError as e:
            raise ProtocolError("invalid UTF-8 string") from e

    def value(self, depth: int = 0) -> Any:
        if depth > MAX_DEPTH:
            raise ProtocolError("value nesting too deep")
        t = self._u8()
        if t == T_NULL:
            return None
        if t == T_BOOL:
            return self._u8() != 0
        if t == T_INT:
            return _I64.unpack(self._take(8))[0]
        if t == T_FLOAT:
            return _F64.unpack(self._take(8))[0]
        if t == T_STRING:
            return self._str()
        if t == T_BYTES:
            return bytes(self._take(self._u32()))
        if t == T_LIST:
            return [self.value(depth + 1) for _ in range(self._count())]
        if t == T_MAP:
            return self.map(depth + 1)
        if t == T_NODE:
            node_id = self._u64()
            labels = [self._str() for _ in range(self._count())]
            return Node(node_id, labels, self.map(depth + 1))
        if t == T_REL:
            rel_id, start, end = self._u64(), self._u64(), self._u64()
            return Relationship(rel_id, start, end, self._str(), self.map(depth + 1))
        if t == T_PATH:
            nodes = [self.value(depth + 1) for _ in range(self._count())]
            rels = [self.value(depth + 1) for _ in range(self._count())]
            if not all(isinstance(n, Node) for n in nodes) or not all(isinstance(r, Relationship) for r in rels):
                raise ProtocolError("a path must contain nodes and relationships")
            if len(nodes) != len(rels) + 1:
                raise ProtocolError("a path needs one more node than relationships")
            return Path(nodes, rels)
        raise ProtocolError(f"unknown value tag {t}")

    def map(self, depth: int = 0) -> dict:
        out = {}
        for _ in range(self._count()):
            k = self._str()
            out[k] = self.value(depth)
        return out


def decode_payload(data: bytes) -> dict:
    d = Decoder(data)
    m = d.map()
    if d.pos != len(d.data):
        raise ProtocolError("trailing bytes after payload")
    return m


# ---------------------------------------------------------------- framing


def encode_frame(message_type: int, request_id: int, payload: bytes) -> bytes:
    if len(payload) > MAX_PAYLOAD:
        raise ValueError("payload too large")
    return _HEADER.pack(PROTOCOL_VERSION, message_type, request_id, len(payload)) + payload


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    chunks = []
    while n:
        chunk = sock.recv(min(n, 1 << 20))
        if not chunk:
            raise ConnectionClosedError("connection closed by the server")
        chunks.append(chunk)
        n -= len(chunk)
    return b"".join(chunks)


def read_frame(sock: socket.socket) -> tuple[int, int, bytes]:
    """Reads one frame: ``(message_type, request_id, payload)``."""
    version, message_type, request_id, length = _HEADER.unpack(_recv_exact(sock, _HEADER.size))
    if version != PROTOCOL_VERSION:
        raise ProtocolError(f"server answered with protocol version {version}, expected {PROTOCOL_VERSION}")
    if length > MAX_PAYLOAD:
        raise ProtocolError(f"payload of {length} bytes exceeds the limit")
    return message_type, request_id, _recv_exact(sock, length) if length else b""

