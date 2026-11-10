"""Values returned by queries."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterator


@dataclass(frozen=True)
class Node:
    id: int
    labels: list[str] = field(default_factory=list)
    properties: dict[str, Any] = field(default_factory=dict)

    def __getitem__(self, key: str) -> Any:
        return self.properties[key]

    def get(self, key: str, default: Any = None) -> Any:
        return self.properties.get(key, default)


@dataclass(frozen=True)
class Relationship:
    id: int
    start_node: int
    end_node: int
    type: str
    properties: dict[str, Any] = field(default_factory=dict)

    def __getitem__(self, key: str) -> Any:
        return self.properties[key]

    def get(self, key: str, default: Any = None) -> Any:
        return self.properties.get(key, default)


@dataclass(frozen=True)
class Stats:
    """Update counters of a statement."""

    nodes_created: int = 0
    nodes_deleted: int = 0
    relationships_created: int = 0
    relationships_deleted: int = 0
    properties_set: int = 0
    labels_added: int = 0
    labels_removed: int = 0

    @classmethod
    def from_map(cls, m: dict[str, Any]) -> "Stats":
        known = cls.__dataclass_fields__
        return cls(**{k: int(v) for k, v in m.items() if k in known})

    @property
    def contains_updates(self) -> bool:
        return self != Stats()


@dataclass
class Result:
    """Rows of a statement plus its update counters.

    ``database`` and ``schema`` are the connection's current ones after the
    statement ran (so ``USE DATABASE`` is visible here).
    """

    columns: list[str]
    rows: list[list[Any]]
    stats: Stats
    database: str = ""
    schema: str = ""
    in_transaction: bool = False

    def __len__(self) -> int:
        return len(self.rows)

    def __iter__(self) -> Iterator[dict[str, Any]]:
        return iter(self.records())

    def records(self) -> list[dict[str, Any]]:
        """Rows as ``{column: value}`` dicts."""
        return [dict(zip(self.columns, row)) for row in self.rows]

    def single(self) -> dict[str, Any] | None:
        """The only record, ``None`` if there are no rows. Raises
        ``ValueError`` if there is more than one."""
        if not self.rows:
            return None
        if len(self.rows) > 1:
            raise ValueError(f"expected at most one row, got {len(self.rows)}")
        return self.records()[0]

    def value(self, column: str | int = 0, default: Any = None) -> Any:
        """A value of the first row, by column name or index."""
        if not self.rows:
            return default
        idx = column if isinstance(column, int) else self.columns.index(column)
        return self.rows[0][idx]

    def values(self, column: str | int = 0) -> list[Any]:
        """One column of every row."""
        idx = column if isinstance(column, int) else self.columns.index(column)
        return [row[idx] for row in self.rows]
