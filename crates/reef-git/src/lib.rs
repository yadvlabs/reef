//! Git storage and access for Reef.
//!
//! Inside a repository we use `gix`. On the wire (upload-pack, receive-pack) we spawn the
//! `git` binary, as every production reef does. Repositories live under one root on
//! shared storage, sharded by id, so renames never move data.

pub mod clone;
pub mod hooks;
pub mod ops;
pub mod read;
pub mod smart;
pub mod store;
pub mod write;

pub use read::{Blob, Commit, Entry, EntryKind, RepoReader, Signature};
pub use store::Store;
