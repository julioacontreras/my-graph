//! Abstract syntax tree for the supported Cypher subset and MyGraph
//! administrative extensions.

use mygraph_core::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Query(Query),
    Admin(AdminCommand),
    Begin,
    Commit,
    Rollback,
}

/// MyGraph administrative extensions (not part of openCypher).
#[derive(Debug, Clone, PartialEq)]
pub enum AdminCommand {
    CreateDatabase {
        name: String,
        if_not_exists: bool,
    },
    DropDatabase {
        name: String,
        if_exists: bool,
    },
    UseDatabase(String),
    CreateSchema {
        name: String,
        if_not_exists: bool,
    },
    DropSchema {
        name: String,
        if_exists: bool,
    },
    UseSchema(String),
    ShowDatabases,
    ShowSchemas,
    /// Equality index on `(label, key)` in the current schema.
    CreateIndex {
        name: Option<String>,
        label: String,
        key: String,
        if_not_exists: bool,
    },
    DropIndex {
        target: IndexTarget,
        if_exists: bool,
    },
    ShowIndexes,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndexTarget {
    Name(String),
    On { label: String, key: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub clauses: Vec<Clause>,
}

impl Query {
    pub fn is_read_only(&self) -> bool {
        self.clauses.iter().all(|c| {
            !matches!(c, Clause::Create { .. } | Clause::Set { .. } | Clause::Remove { .. } | Clause::Delete { .. })
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Clause {
    Match { optional: bool, patterns: Vec<PathPattern>, filter: Option<Expr> },
    Create { patterns: Vec<PathPattern> },
    Set { items: Vec<SetItem> },
    Remove { items: Vec<RemoveItem> },
    Delete { detach: bool, exprs: Vec<Expr> },
    Unwind { expr: Expr, variable: String },
    With { projection: Projection, filter: Option<Expr> },
    Return(Projection),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PathPattern {
    /// `p = (a)-->(b)`: binds the matched path to `p`.
    pub name: Option<String>,
    pub shortest: Option<Shortest>,
    pub start: NodePattern,
    pub steps: Vec<(RelPattern, NodePattern)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortest {
    /// `shortestPath(...)`: one shortest path per pair of end nodes.
    One,
    /// `allShortestPaths(...)`: every path of the minimum length.
    All,
}

impl PathPattern {
    /// Variables introduced by the pattern, in order of appearance.
    pub fn variables(&self) -> Vec<&str> {
        let all = std::iter::once(&self.name)
            .chain(std::iter::once(&self.start.variable))
            .chain(self.steps.iter().flat_map(|(r, n)| [&r.variable, &n.variable]));
        let mut vars: Vec<&str> = Vec::new();
        for v in all.flatten() {
            if !vars.contains(&v.as_str()) {
                vars.push(v);
            }
        }
        vars
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    pub variable: Option<String>,
    pub labels: Vec<String>,
    pub properties: Option<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RelPattern {
    pub variable: Option<String>,
    pub types: Vec<String>,
    pub properties: Option<Expr>,
    pub direction: Direction,
    /// `*min..max` for variable-length relationships; the variable then binds
    /// to a list of relationships.
    pub length: Option<VarLength>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarLength {
    pub min: u32,
    /// `None`: no upper bound (relationship uniqueness still ends the search).
    pub max: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SetItem {
    Property {
        variable: String,
        key: String,
        value: Expr,
    },
    /// `SET n = {map}`
    Replace {
        variable: String,
        value: Expr,
    },
    /// `SET n += {map}`
    Merge {
        variable: String,
        value: Expr,
    },
    Labels {
        variable: String,
        labels: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RemoveItem {
    Property { variable: String, key: String },
    Labels { variable: String, labels: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    pub distinct: bool,
    /// `RETURN *` / `WITH *`
    pub star: bool,
    pub items: Vec<ProjectionItem>,
    pub order_by: Vec<SortItem>,
    pub skip: Option<Expr>,
    pub limit: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionItem {
    pub expr: Expr,
    /// Column name: the alias, or the expression's source text.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SortItem {
    pub expr: Expr,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Parameter(String),
    Variable(String),
    Property(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    List(Vec<Expr>),
    Map(Vec<(String, Expr)>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    IsNull { expr: Box<Expr>, negated: bool },
    HasLabels(Box<Expr>, Vec<String>),
    FunctionCall { name: String, args: Vec<Expr>, distinct: bool },
    CountStar,
    Case { operand: Option<Box<Expr>>, branches: Vec<(Expr, Expr)>, default: Option<Box<Expr>> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
    Pos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    Xor,
    And,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    In,
    StartsWith,
    EndsWith,
    Contains,
}

pub const AGGREGATE_FUNCTIONS: &[&str] = &["count", "sum", "avg", "min", "max", "collect"];

pub fn is_aggregate_function(name: &str) -> bool {
    AGGREGATE_FUNCTIONS.iter().any(|f| f.eq_ignore_ascii_case(name))
}

impl Expr {
    /// Whether the expression contains an aggregate function call.
    pub fn contains_aggregate(&self) -> bool {
        let mut found = false;
        self.walk(&mut |e| {
            found |= matches!(e, Expr::CountStar)
                || matches!(e, Expr::FunctionCall { name, .. } if is_aggregate_function(name));
        });
        found
    }

    /// Pre-order traversal of the expression tree.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        match self {
            Expr::Literal(_) | Expr::Parameter(_) | Expr::Variable(_) | Expr::CountStar => {}
            Expr::Property(e, _) | Expr::Unary(_, e) | Expr::HasLabels(e, _) => e.walk(f),
            Expr::IsNull { expr, .. } => expr.walk(f),
            Expr::Index(a, b) | Expr::Binary(_, a, b) => {
                a.walk(f);
                b.walk(f);
            }
            Expr::List(items) | Expr::FunctionCall { args: items, .. } => items.iter().for_each(|e| e.walk(f)),
            Expr::Map(entries) => entries.iter().for_each(|(_, e)| e.walk(f)),
            Expr::Case { operand, branches, default } => {
                if let Some(o) = operand {
                    o.walk(f);
                }
                for (w, t) in branches {
                    w.walk(f);
                    t.walk(f);
                }
                if let Some(d) = default {
                    d.walk(f);
                }
            }
        }
    }
}
