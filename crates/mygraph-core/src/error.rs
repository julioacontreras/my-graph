use thiserror::Error;

/// Error type shared by every MyGraph component.
#[derive(Debug, Error)]
pub enum MyGraphError {
    #[error("syntax error at line {line}, column {column}: {message}")]
    Syntax { message: String, line: u32, column: u32 },
    #[error("semantic error: {0}")]
    Semantic(String),
    #[error("type error: {0}")]
    Type(String),
    #[error("execution error: {0}")]
    Execution(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("constraint violation: {0}")]
    ConstraintViolation(String),
    #[error("transaction error: {0}")]
    Transaction(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("corrupted data: {0}")]
    Corruption(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MyGraphError>;
