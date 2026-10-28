//! Domain layer of tokred.
//!
//! This crate performs no I/O. It holds the domain types, the error enums, the
//! `tokred.toml` configuration model, the path resolution rules and the *ports*
//! (traits) that the use-case crates consume and the adapters implement.
//!
//! Dependencies always point inwards: use cases and adapters depend on this crate,
//! never the other way around.

pub mod config;
pub mod error;
pub mod hook;
pub mod paths;
pub mod ports;
pub mod types;

pub use config::{Config, Mode};
pub use error::{ConfigError, FsError, StoreError};
pub use hook::HookEvent;
pub use paths::DataLayout;
pub use ports::{Clock, FileSystem, ManifestRepo};
pub use types::{ManifestEntry, Timestamp};
