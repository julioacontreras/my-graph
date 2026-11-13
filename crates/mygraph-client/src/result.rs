use mygraph_core::{PropertyMap, Value};

/// Update counters of a query, summed over all its statements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub nodes_created: u64,
    pub nodes_deleted: u64,
    pub relationships_created: u64,
    pub relationships_deleted: u64,
    pub properties_set: u64,
    pub labels_added: u64,
    pub labels_removed: u64,
}

impl Stats {
    pub fn contains_updates(&self) -> bool {
        *self != Stats::default()
    }

    pub(crate) fn from_map(m: &PropertyMap) -> Self {
        let n = |k: &str| match m.get(k) {
            Some(Value::Integer(i)) => (*i).max(0) as u64,
            _ => 0,
        };
        Stats {
            nodes_created: n("nodes_created"),
            nodes_deleted: n("nodes_deleted"),
            relationships_created: n("relationships_created"),
            relationships_deleted: n("relationships_deleted"),
            properties_set: n("properties_set"),
            labels_added: n("labels_added"),
            labels_removed: n("labels_removed"),
        }
    }
}

/// Rows of the last statement of a query, plus the session state after it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub stats: Stats,
    /// Current database of the session after the query.
    pub database: String,
    /// Current schema of the session after the query.
    pub schema: String,
    /// Whether an explicit transaction is open after the query.
    pub in_transaction: bool,
}

impl QueryResult {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// Value at `row` in the column named `column`.
    pub fn value(&self, row: usize, column: &str) -> Option<&Value> {
        let c = self.column_index(column)?;
        self.rows.get(row)?.get(c)
    }

    /// The first value of the first row, for queries such as `RETURN count(*)`.
    pub fn single(&self) -> Option<&Value> {
        self.rows.first()?.first()
    }

    /// Iterates rows as `(column, value)` pairs.
    pub fn records(&self) -> impl Iterator<Item = Vec<(&str, &Value)>> + '_ {
        self.rows.iter().map(|r| self.columns.iter().map(String::as_str).zip(r.iter()).collect())
    }
}
