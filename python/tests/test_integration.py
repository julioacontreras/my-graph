"""End-to-end tests against a real mygraph-server.

The server is built with cargo and started on a free port. Set
MYGRAPH_SERVER_BIN to use an existing binary instead. Skipped when neither
cargo nor the binary is available.
"""

import os
import shutil
import subprocess
import threading
import time
import unittest
from pathlib import Path

import mygraph
from mygraph import CypherSyntaxError, Node, NotFoundError, Relationship, TransactionError

REPO = Path(__file__).resolve().parents[2]


def _server_binary():
    explicit = os.environ.get("MYGRAPH_SERVER_BIN")
    if explicit:
        return explicit
    if shutil.which("cargo") is None:
        return None
    subprocess.run(["cargo", "build", "-q", "-p", "mygraph-server"], cwd=REPO, check=True)
    return str(REPO / "target" / "debug" / "mygraph-server")


class ServerTest(unittest.TestCase):
    proc = None
    port = None

    @classmethod
    def setUpClass(cls):
        binary = _server_binary()
        if binary is None:
            raise unittest.SkipTest("cargo not found and MYGRAPH_SERVER_BIN not set")
        cls.proc = subprocess.Popen(
            [binary, "start", "--port", "0"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True
        )
        line = cls.proc.stdout.readline()
        if "listening on" not in line:
            cls.proc.kill()
            raise RuntimeError(f"server did not start: {line!r}")
        cls.port = int(line.split("listening on ")[1].split()[0].rsplit(":", 1)[1])

    @classmethod
    def tearDownClass(cls):
        if cls.proc:
            cls.proc.terminate()
            cls.proc.wait(timeout=10)

    def setUp(self):
        self.db = mygraph.connect("127.0.0.1", self.port)
        # Each test gets its own database so tests do not see each other's data.
        self.dbname = f"t_{self._testMethodName}"
        self.db.drop_database(self.dbname)
        self.db.create_database(self.dbname)
        self.db.use_database(self.dbname)

    def tearDown(self):
        self.db.close()

    # ------------------------------------------------------------ basics

    def test_handshake_and_ping(self):
        self.assertEqual(self.db.server_info["protocol"], 1)
        self.db.ping()

    def test_run_with_params(self):
        r = self.db.run("RETURN $a + 1 AS x, $s AS s, $l AS l", {"a": 41}, s="hi", l=[1, 2.5, None])
        self.assertEqual(r.single(), {"x": 42, "s": "hi", "l": [1, 2.5, None]})
        self.assertEqual(r.database, self.dbname)

    def test_index_and_paths(self):
        self.db.run("CREATE INDEX ON :Step(n)")
        self.db.run("UNWIND range(0, 4) AS i CREATE (:Step {n: i})")
        self.db.run("MATCH (a:Step), (b:Step) WHERE b.n = a.n + 1 CREATE (a)-[:NEXT]->(b)")
        self.assertEqual(self.db.run("SHOW INDEXES").single()["entries"], 5)
        r = self.db.run(
            "MATCH (a:Step {n: 0}), (b:Step {n: 4}), p = shortestPath((a)-[:NEXT*]->(b)) RETURN p, length(p) AS len"
        ).single()
        path = r["p"]
        self.assertIsInstance(path, mygraph.Path)
        self.assertEqual((len(path), r["len"]), (4, 4))
        self.assertEqual([n["n"] for n in path.nodes], [0, 1, 2, 3, 4])
        self.assertEqual([rel.type for rel in path.relationships], ["NEXT"] * 4)
        far = self.db.run("MATCH (:Step {n: 1})-[:NEXT*2..]->(x) RETURN x.n AS n ORDER BY n").records()
        self.assertEqual([row["n"] for row in far], [3, 4])

    # ------------------------------------------------------------ nodes

    def test_node_crud(self):
        alice = self.db.create_node(["User", "Admin"], {"name": "Alice", "age": 25})
        self.assertIsInstance(alice, Node)
        self.assertEqual(sorted(alice.labels), ["Admin", "User"])
        self.assertEqual(alice["age"], 25)
        self.db.create_node("User", {"name": "Bob", "age": 30})

        self.assertEqual([n["name"] for n in self.db.find_nodes("User", order_by="age")], ["Alice", "Bob"])
        self.assertEqual([n["name"] for n in self.db.find_nodes("User", {"name": "Bob"})], ["Bob"])
        self.assertEqual(len(self.db.find_nodes("User", limit=1)), 1)
        self.assertEqual(self.db.count_nodes("Admin"), 1)
        self.assertEqual(self.db.get_node(alice.id), alice)

        updated = self.db.update_node(alice.id, {"age": 26, "city": "Madrid"}, remove_labels="Admin")
        self.assertEqual(updated.properties, {"name": "Alice", "age": 26, "city": "Madrid"})
        self.assertEqual(updated.labels, ["User"])
        replaced = self.db.update_node(alice.id, {"name": "Alicia"}, replace=True)
        self.assertEqual(replaced.properties, {"name": "Alicia"})
        self.assertIsNone(self.db.update_node(10**9, {"x": 1}))

        self.assertTrue(self.db.delete_node(alice.id))
        self.assertFalse(self.db.delete_node(alice.id))
        self.assertIsNone(self.db.get_node(alice.id))
        self.assertEqual(self.db.delete_nodes("User"), 1)

    def test_odd_identifiers_are_quoted(self):
        n = self.db.create_node("Weird `Label`", {"key with space": 1})
        self.assertEqual(n.labels, ["Weird `Label`"])
        self.assertEqual(len(self.db.find_nodes("Weird `Label`", {"key with space": 1})), 1)

    # ------------------------------------------------------------ relationships

    def test_relationship_crud(self):
        a = self.db.create_node("User", {"name": "A"})
        b = self.db.create_node("User", {"name": "B"})
        rel = self.db.create_relationship(a.id, b.id, "KNOWS", {"since": 2020})
        self.assertIsInstance(rel, Relationship)
        self.assertEqual((rel.start_node, rel.end_node, rel.type), (a.id, b.id, "KNOWS"))
        self.assertIsNone(self.db.create_relationship(a.id, 10**9, "KNOWS"))

        self.assertEqual(self.db.find_relationships("KNOWS", start_id=a.id), [rel])
        self.assertEqual(self.db.find_relationships("KNOWS", start_id=b.id), [])
        self.assertEqual(self.db.get_relationship(rel.id), rel)
        self.assertEqual(self.db.update_relationship(rel.id, {"since": 2021})["since"], 2021)
        self.assertTrue(self.db.delete_relationship(rel.id))
        self.assertEqual(self.db.find_relationships(), [])

    def test_detach_delete(self):
        a = self.db.create_node("User")
        b = self.db.create_node("User")
        self.db.create_relationship(a.id, b.id, "KNOWS")
        with self.assertRaises(mygraph.QueryError):
            self.db.delete_node(a.id, detach=False)
        self.assertTrue(self.db.delete_node(a.id))
        self.assertEqual(self.db.find_relationships(), [])

    # ------------------------------------------------------------ transactions

    def test_transaction_commit_and_rollback(self):
        with self.db.transaction() as tx:
            tx.create_node("T", {"v": 1})
        self.assertEqual(self.db.count_nodes("T"), 1)

        with self.assertRaises(RuntimeError):
            with self.db.transaction() as tx:
                tx.create_node("T", {"v": 2})
                raise RuntimeError("boom")
        self.assertEqual(self.db.count_nodes("T"), 1)

        tx = self.db.begin()
        tx.run("CREATE (:T {v: 3})")
        tx.rollback()
        self.assertEqual(self.db.count_nodes("T"), 1)
        with self.assertRaises(TransactionError):
            tx.commit()

    def test_open_transaction_is_rolled_back_on_close(self):
        other = mygraph.connect("127.0.0.1", self.port)
        other.use_database(self.dbname)
        other.begin().create_node("T")
        other.close()
        self.assertEqual(self.db.count_nodes("T"), 0)

    # ------------------------------------------------------------ errors and sessions

    def test_errors(self):
        with self.assertRaises(CypherSyntaxError) as ctx:
            self.db.run("MATCH (n RETURN n")
        self.assertIsNotNone(ctx.exception.line)
        with self.assertRaises(NotFoundError):
            self.db.use_database("does_not_exist")
        self.db.ping()  # connection still usable

    def test_use_database_is_per_connection(self):
        other = mygraph.connect("127.0.0.1", self.port)
        try:
            self.assertNotEqual(other.run("RETURN 1").database, self.dbname)
        finally:
            other.close()

    def test_concurrent_clients(self):
        errors = []

        def work(i):
            try:
                with mygraph.connect("127.0.0.1", self.port) as c:
                    c.use_database(self.dbname)
                    for j in range(20):
                        c.create_node("C", {"i": i, "j": j})
            except Exception as e:  # pragma: no cover
                errors.append(e)

        threads = [threading.Thread(target=work, args=(i,)) for i in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(errors, [])
        self.assertEqual(self.db.count_nodes("C"), 80)

    def test_closed_client(self):
        self.db.close()
        with self.assertRaises(mygraph.ConnectionClosedError):
            self.db.ping()
        self.db = mygraph.connect("127.0.0.1", self.port)


if __name__ == "__main__":
    unittest.main()
