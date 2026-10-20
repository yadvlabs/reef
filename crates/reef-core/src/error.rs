use std::fmt;

/// The one error type that crosses crate boundaries. API layers map it to HTTP.
#[derive(Debug)]
pub enum Error {
    NotFound,
    Unauthorized(String),
    Forbidden(String),
    /// A user-correctable problem with the request. Carries GitHub-style field errors.
    Validation {
        message: String,
        errors: Vec<FieldError>,
    },
    Conflict(String),
    Internal(Box<dyn std::error::Error + Send + Sync + 'static>),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldError {
    pub resource: String,
    pub field: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn validation(message: impl Into<String>) -> Self {
        Error::Validation { message: message.into(), errors: Vec::new() }
    }
    pub fn field(resource: &str, field: &str, code: &str, message: impl Into<String>) -> Self {
        Error::Validation {
            message: "Validation Failed".into(),
            errors: vec![FieldError { resource: resource.into(), field: field.into(), code: code.into(), message: Some(message.into()) }],
        }
    }
    pub fn internal<E: std::error::Error + Send + Sync + 'static>(e: E) -> Self {
        Error::Internal(Box::new(e))
    }
    pub fn msg(m: impl Into<String>) -> Self {
        Error::Internal(m.into().into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "Not Found"),
            Error::Unauthorized(m) => write!(f, "{m}"),
            Error::Forbidden(m) => write!(f, "{m}"),
            Error::Validation { message, .. } => write!(f, "{message}"),
            Error::Conflict(m) => write!(f, "{m}"),
            Error::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Internal(Box::new(e))
    }
}
