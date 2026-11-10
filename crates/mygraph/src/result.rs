use mygraph_core::Value;
use mygraph_cypher::{ExecResult, QueryStats};

/// Rows returned by a statement, plus update counters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub stats: QueryStats,
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

    /// Iterates rows as `(column, value)` pairs.
    pub fn records(&self) -> impl Iterator<Item = Vec<(&str, &Value)>> + '_ {
        self.rows.iter().map(|r| self.columns.iter().map(String::as_str).zip(r.iter()).collect())
    }
}

impl From<ExecResult> for QueryResult {
    fn from(r: ExecResult) -> Self {
        QueryResult { columns: r.columns, rows: r.rows, stats: r.stats }
    }
}
