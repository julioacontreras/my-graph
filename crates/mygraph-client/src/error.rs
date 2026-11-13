use std::fmt;
use std::io;

/// Errors returned by the client.
#[derive(Debug)]
pub enum Error {
    /// The connection could not be opened, was lost, or is already closed.
    /// A client that returned this error cannot be used again.
    Connection(io::Error),
    /// The server sent something that does not follow protocol v1, or
    /// rejected a malformed request. The connection is closed.
    Protocol(String),
    /// The server could not run a statement. The connection stays usable.
    Query(QueryError),
}

/// An `ERROR` message from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    /// Engine error kind: `Syntax`, `Semantic`, `Type`, `Execution`,
    /// `NotFound`, `AlreadyExists`, `InvalidName`, `Unsupported`,
    /// `ConstraintViolation`, `Transaction`, `Storage`, `Corruption` or `Io`.
    pub code: String,
    pub message: String,
    /// Position of a syntax error (1-based).
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl QueryError {
    pub fn is_syntax(&self) -> bool {
        self.code == "Syntax"
    }

    pub fn is_not_found(&self) -> bool {
        self.code == "NotFound"
    }

    pub fn is_already_exists(&self) -> bool {
        self.code == "AlreadyExists"
    }

    /// A write broke a unique or not-null constraint; the statement was undone.
    pub fn is_constraint_violation(&self) -> bool {
        self.code == "ConstraintViolation"
    }

    pub fn is_transaction(&self) -> bool {
        self.code == "Transaction"
    }
}

impl Error {
    /// The server's error, if this is a query error.
    pub fn as_query(&self) -> Option<&QueryError> {
        match self {
            Error::Query(q) => Some(q),
            _ => None,
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for QueryError {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Connection(e) => write!(f, "connection error: {e}"),
            Error::Protocol(m) => write!(f, "protocol error: {m}"),
            Error::Query(q) => q.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Connection(e) => Some(e),
            Error::Query(q) => Some(q),
            Error::Protocol(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Connection(e)
    }
}

impl From<QueryError> for Error {
    fn from(e: QueryError) -> Self {
        Error::Query(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
