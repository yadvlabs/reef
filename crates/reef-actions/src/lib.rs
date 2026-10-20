//! The GitHub Actions workflow model, independent of storage and transport: parse workflow
//! YAML, decide which workflows an event triggers, expand job matrices, and evaluate the
//! `${{ }}` expression language for job-level `if`, `runs-on`, names and outputs.

pub mod expr;
pub mod glob;
pub mod matrix;
pub mod trigger;
pub mod workflow;

pub use expr::{Value, evaluate, interpolate};
pub use workflow::{Job, Step, StepAction, Workflow, parse};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("workflow is not valid YAML: {0}")]
    Yaml(String),
    #[error("{0}")]
    Invalid(String),
    #[error("expression error: {0}")]
    Expr(String),
}

pub type Result<T> = std::result::Result<T, Error>;
