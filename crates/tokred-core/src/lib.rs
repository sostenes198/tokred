//! Domain layer of tokred.
//!
//! This crate performs no I/O. It holds the domain types, the error enums, the
//! `tokred.toml` configuration model, the path resolution rules and the *ports*
//! (traits) that the use-case crates consume and the adapters implement.
//!
//! Dependencies always point inwards: use cases and adapters depend on this crate,
//! never the other way around.

pub mod cache;
pub mod compression;
pub mod config;
pub mod error;
pub mod hook;
pub mod hook_event;
pub mod meta;
pub mod paths;
pub mod ports;
pub mod pricing;
pub mod prompts;
pub mod proxy;
pub mod release;
pub mod routing;
pub mod session;
pub mod types;
pub mod usage;

pub use config::{Config, Mode};
pub use error::{ConfigError, FsError, StoreError};
pub use hook::HookEvent;
pub use paths::DataLayout;
pub use ports::{Clock, FileSystem, ManifestRepo};
pub use types::{ManifestEntry, Timestamp};
