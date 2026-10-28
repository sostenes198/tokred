//! Use cases behind `tokred configure`, `tokred unconfigure` and `tokred doctor`.
//!
//! * [`settings`] – pure, non-destructive merge of tokred hooks into `settings.json`;
//! * [`agents`] – the six subagent templates and the rules for installing/updating them;
//! * [`mod@configure`], [`mod@unconfigure`], [`mod@doctor`] – the use cases, written against the ports of
//!   `tokred-core` so tests can run on temporary directories;
//! * [`fs`] – the `std::fs` adapter of the `FileSystem` port.

pub mod agents;
pub mod configure;
pub mod doctor;
pub mod error;
pub mod fs;
pub mod settings;
pub mod unconfigure;

pub use configure::{
    AgentPlan, ConfigureInput, ConfigurePlan, ConfigureReport, apply_configure, plan_configure,
};
pub use doctor::{DoctorReport, doctor};
pub use error::{InstallError, MergeError};
pub use fs::StdFileSystem;
pub use unconfigure::{UnconfigureReport, unconfigure};
