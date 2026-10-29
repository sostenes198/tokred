//! Version check, install-method detection and self-update for tokred.
//!
//! * [`detect`] and [`upgrade_hint`] tell how tokred was installed and how to upgrade it.
//! * [`run_check`], [`check_due`], [`should_check`] and [`notice`] implement the once-a-day
//!   "new version available" check. It is never run from a hook.
//! * [`self_update`] downloads, verifies and swaps the binary (script installs only).
//! * [`needs_reconfigure`] and [`mark_configured`] detect that hooks and agents must be reapplied
//!   after the binary changed version.
//!
//! Every port (release source, downloader, file system, meta store, clock) is a parameter, so the
//! logic is tested without network or real files. The `http` module holds the `ureq` adapters.

mod check;
mod install;
mod reconfigure;
mod swap;
mod version;

pub mod http;

pub use check::{
    NewVersion, VERSION_CHECK_AT, VERSION_CHECK_LATEST, check_due, notice, run_check, should_check,
    should_self_update,
};
pub use install::{InstallMethod, detect, install_bin_dir, upgrade_hint};
pub use reconfigure::{TOKRED_VERSION, mark_configured, needs_reconfigure};
pub use swap::{
    BinarySwap, Downloader, SelfUpdated, StdBinarySwap, UpdateError, asset_name,
    cleanup_old_binary, current_target, old_binary_path, release_asset_url, self_update,
    target_triple,
};
pub use version::is_newer;
