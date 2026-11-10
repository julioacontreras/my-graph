"""Node and relationship helpers built on ``run``.

The helpers generate Cypher. Values always travel as parameters. Labels,
relationship types and property keys are written as backtick-quoted
identifiers, so any name is safe to pass.
"""

from __future__ import annotations

from typing import Any, Iterable, Mapping, Sequence

from .types import Node, Relationship, Result


def quote(name: str) -> str:
    """Backtick-quotes an identifier (label, type or property key)."""
    if not isinstance(name, str) or not name:
        raise ValueError(f"invalid identifier: {name!r}")
    return "`" + name.replace("`", "``") + "`"


def _labels(labels: str | Sequence[str] | None) -> str:
    if labels is None:
        return ""
    if isinstance(labels, str):
        labels = [labels]
    return "".join(":" + quote(label) for label in labels)


def _where(var: str, properties: Mapping[str, Any] | None, params: dict, extra: Iterable[str] = ()) -> str:
    conds = list(extra)
    for i, (key, value) in enumerate((properties or {}).items()):
        if value is None:
            conds.append(f"{var}.{quote(key)} IS NULL")
        else:
            params[f"p{i}"] = value
            conds.append(f"{var}.{quote(key)} = $p{i}")
    return " WHERE " + " AND ".join(conds) if conds else ""


def _paging(skip: int | None, limit: int | None) -> str:
    out = ""
    for word, n in (("SKIP", skip), ("LIMIT", limit)):
        if n is not None:
            if isinstance(n, bool) or not isinstance(n, int) or n < 0:
                raise ValueError(f"{word.lower()} must be a non-negative int")
            out += f" {word} {n}"
    return out


class CypherOps:
    """CRUD helpers. Subclasses provide ``run``."""

    def run(self, query: str, params: Mapping[str, Any] | None = None, **kwargs: Any) -> Result:
        raise NotImplementedError

    # ------------------------------------------------------------ nodes

    def create_node(self, labels: str | Sequence[str] | None = None, properties: Mapping[str, Any] | None = None) -> Node:
        """Creates a node and returns it."""
        r = self.run(f"CREATE (n{_labels(labels)}) SET n = $props RETURN n", {"props": dict(properties or {})})
        return r.value()

    def get_node(self, node_id: int) -> Node | None:
        """The node with that id, or ``None``."""
        return self.run("MATCH (n) WHERE id(n) = $id RETURN n", {"id": node_id}).value()

    def find_nodes(
        self,
        labels: str | Sequence[str] | None = None,
        properties: Mapping[str, Any] | None = None,
        *,
        skip: int | None = None,
        limit: int | None = None,
        order_by: str | None = None,
        descending: bool = False,
    ) -> list[Node]:
        """Nodes with all the given labels whose properties equal the given
        values (``None`` matches a missing property)."""
        params: dict[str, Any] = {}
        q = f"MATCH (n{_labels(labels)}){_where('n', properties, params)} RETURN n"
        if order_by is not None:
            q += f" ORDER BY n.{quote(order_by)}" + (" DESC" if descending else "")
        return self.run(q + _paging(skip, limit), params).values(0)

    def count_nodes(self, labels: str | Sequence[str] | None = None, properties: Mapping[str, Any] | None = None) -> int:
        params: dict[str, Any] = {}
        q = f"MATCH (n{_labels(labels)}){_where('n', properties, params)} RETURN count(n)"
        return self.run(q, params).value()

    def update_node(
        self,
        node_id: int,
        properties: Mapping[str, Any] | None = None,
        *,
        replace: bool = False,
        add_labels: str | Sequence[str] | None = None,
        remove_labels: str | Sequence[str] | None = None,
    ) -> Node | None:
        """Updates a node and returns it, or ``None`` if it does not exist.

        By default ``properties`` are merged (``SET n += $props``; a ``None``
        value removes that property). With ``replace=True`` they replace all
        existing properties (``SET n = $props``).
        """
        clauses = []
        params: dict[str, Any] = {"id": node_id}
        if properties is not None or replace:
            params["props"] = dict(properties or {})
            clauses.append("SET n = $props" if replace else "SET n += $props")
        if add_labels:
            clauses.append(f"SET n{_labels(add_labels)}")
        if remove_labels:
            clauses.append(f"REMOVE n{_labels(remove_labels)}")
        q = "MATCH (n) WHERE id(n) = $id " + " ".join(clauses) + " RETURN n"
        return self.run(q, params).value()

    def delete_node(self, node_id: int, *, detach: bool = True) -> bool:
        """Deletes a node. With ``detach=True`` (default) its relationships
        are deleted too; otherwise a node with relationships raises.
        Returns whether a node was deleted."""
        verb = "DETACH DELETE" if detach else "DELETE"
        r = self.run(f"MATCH (n) WHERE id(n) = $id {verb} n", {"id": node_id})
        return r.stats.nodes_deleted > 0

    def delete_nodes(
        self,
        labels: str | Sequence[str] | None = None,
        properties: Mapping[str, Any] | None = None,
        *,
        detach: bool = True,
    ) -> int:
        """Deletes every matching node. Returns how many were deleted."""
        params: dict[str, Any] = {}
        verb = "DETACH DELETE" if detach else "DELETE"
        r = self.run(f"MATCH (n{_labels(labels)}){_where('n', properties, params)} {verb} n", params)
        return r.stats.nodes_deleted

    # ------------------------------------------------------------ relationships

    def create_relationship(
        self,
        start_id: int,
        end_id: int,
        rel_type: str,
        properties: Mapping[str, Any] | None = None,
    ) -> Relationship | None:
        """Creates ``(start)-[:rel_type]->(end)`` and returns it, or ``None``
        if either node does not exist."""
        q = (
            "MATCH (a) WHERE id(a) = $start MATCH (b) WHERE id(b) = $end "
            f"CREATE (a)-[r:{quote(rel_type)}]->(b) SET r = $props RETURN r"
        )
        params = {"start": start_id, "end": end_id, "props": dict(properties or {})}
        return self.run(q, params).value()

    def get_relationship(self, rel_id: int) -> Relationship | None:
        return self.run("MATCH ()-[r]->() WHERE id(r) = $id RETURN r", {"id": rel_id}).value()

    def find_relationships(
        self,
        rel_type: str | None = None,
        properties: Mapping[str, Any] | None = None,
        *,
        start_id: int | None = None,
        end_id: int | None = None,
        skip: int | None = None,
        limit: int | None = None,
    ) -> list[Relationship]:
        """Relationships of a type (any if ``None``) whose properties equal
        the given values, optionally from/to specific nodes."""
        params: dict[str, Any] = {}
        extra = []
        if start_id is not None:
            params["start"] = start_id
            extra.append("id(a) = $start")
        if end_id is not None:
            params["end"] = end_id
            extra.append("id(b) = $end")
        t = f":{quote(rel_type)}" if rel_type else ""
        q = f"MATCH (a)-[r{t}]->(b){_where('r', properties, params, extra)} RETURN r"
        return self.run(q + _paging(skip, limit), params).values(0)

    def update_relationship(
        self, rel_id: int, properties: Mapping[str, Any], *, replace: bool = False
    ) -> Relationship | None:
        """Merges (or with ``replace=True`` replaces) a relationship's
        properties. Returns it, or ``None`` if it does not exist."""
        op = "=" if replace else "+="
        q = f"MATCH ()-[r]->() WHERE id(r) = $id SET r {op} $props RETURN r"
        return self.run(q, {"id": rel_id, "props": dict(properties)}).value()

    def delete_relationship(self, rel_id: int) -> bool:
        r = self.run("MATCH ()-[r]->() WHERE id(r) = $id DELETE r", {"id": rel_id})
        return r.stats.relationships_deleted > 0
