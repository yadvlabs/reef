//! Shared primitives for Reef: identifiers, time, errors, tokens, and name rules.

pub mod error;
pub mod id;
pub mod names;
pub mod time;
pub mod token;

pub use error::{Error, Result};
pub use id::{Id, IdGen};
