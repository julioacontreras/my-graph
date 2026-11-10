"""Codec and framing tests; no server needed."""

import struct
import unittest

from mygraph import Node, ProtocolError, Relationship
from mygraph import _protocol as p


def roundtrip(v):
    buf = bytearray()
    p.encode_value(buf, v)
    d = p.Decoder(bytes(buf))
    out = d.value()
    assert d.pos == len(buf)
    return out


class CodecTest(unittest.TestCase):
    def test_scalars_and_containers(self):
        v = [None, True, False, -42, 2**63 - 1, 1.5, "héllo", b"\x00\xff", {"k": [1, 2.0]}, []]
        self.assertEqual(roundtrip(v), v)

    def test_int_float_distinction(self):
        self.assertIs(type(roundtrip(1)), int)
        self.assertIs(type(roundtrip(1.0)), float)
        self.assertIs(roundtrip(True), True)

    def test_tuple_becomes_list(self):
        self.assertEqual(roundtrip((1, 2)), [1, 2])

    def test_byte_layout_matches_spec(self):
        buf = bytearray()
        p.encode_value(buf, {"a": 1})
        self.assertEqual(bytes(buf), bytes([p.T_MAP]) + struct.pack("<I", 1) + struct.pack("<I", 1) + b"a"
                         + bytes([p.T_INT]) + struct.pack("<q", 1))

    def test_decode_node_and_relationship(self):
        node = (bytes([p.T_NODE]) + struct.pack("<Q", 7) + struct.pack("<I", 1) + struct.pack("<I", 4) + b"User"
                + struct.pack("<I", 0))
        self.assertEqual(p.Decoder(node).value(), Node(7, ["User"], {}))
        rel = (bytes([p.T_REL]) + struct.pack("<QQQ", 1, 7, 8) + struct.pack("<I", 5) + b"KNOWS"
               + struct.pack("<I", 0))
        self.assertEqual(p.Decoder(rel).value(), Relationship(1, 7, 8, "KNOWS", {}))

    def test_rejects_unsendable_values(self):
        for bad in (2**63, object(), {1: "x"}, Node(1)):
            with self.assertRaises((TypeError, ValueError)):
                p.encode_value(bytearray(), bad)

    def test_rejects_malformed_input(self):
        for data in (b"\x2a", bytes([p.T_INT, 1]), bytes([p.T_LIST]) + b"\xff\xff\xff\xff"):
            with self.assertRaises(ProtocolError):
                p.Decoder(data).value()
        with self.assertRaises(ProtocolError):
            p.decode_payload(p.encode_payload({"a": 1}) + b"\x00")

    def test_frame_header(self):
        frame = p.encode_frame(p.QUERY, 9, b"xyz")
        self.assertEqual(frame[:10], struct.pack("<BBII", 1, 0x10, 9, 3))


if __name__ == "__main__":
    unittest.main()
