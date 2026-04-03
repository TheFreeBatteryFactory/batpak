use serde::{Deserialize, Serialize};
use std::fmt;

/// OutcomeError: structured error with kind, message, optional compensation.
/// [SPEC:src/outcome/error.rs]

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutcomeError {
    /// Classification of the error.
    pub kind: ErrorKind,
    /// Human-readable description of what went wrong.
    pub message: String,
    /// Optional compensation action to run when this error occurs.
    pub compensation: Option<super::wait::CompensationAction>,
    /// Whether the caller may safely retry the operation.
    pub retryable: bool,
}

/// ErrorKind: 8 domain kinds + Custom(u16) for product extension.
/// Products extend via Custom(u16) — same category:type encoding as EventKind.
/// [SPEC:src/outcome/error.rs — ErrorKind]

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A requested resource does not exist.
    NotFound,
    /// An operation conflicts with existing state.
    Conflict,
    /// Input failed validation rules.
    Validation,
    /// A gate or policy explicitly rejected the operation.
    PolicyRejection,
    /// A persistence or storage layer failure.
    StorageError,
    /// An operation exceeded its time limit.
    Timeout,
    /// A serialization or deserialization failure.
    Serialization,
    /// An unexpected internal error.
    Internal,
    /// A product-defined error kind identified by a numeric code.
    Custom(u16),
}

impl ErrorKind {
    /// Returns true if this error kind is considered retryable (`StorageError` or `Timeout`).
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::StorageError | Self::Timeout)
    }

    /// Returns true if this error kind is a domain error (`NotFound`, `Conflict`, `Validation`, or `PolicyRejection`).
    pub fn is_domain(&self) -> bool {
        matches!(
            self,
            Self::NotFound | Self::Conflict | Self::Validation | Self::PolicyRejection
        )
    }

    /// Returns true if this error kind is operational (`StorageError`, `Timeout`, `Serialization`, or `Internal`).
    pub fn is_operational(&self) -> bool {
        matches!(
            self,
            Self::StorageError | Self::Timeout | Self::Serialization | Self::Internal
        )
    }
}

impl fmt::Display for OutcomeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {}", self.kind, self.message)
    }
}
impl std::error::Error for OutcomeError {}
