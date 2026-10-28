//! The once-a-day "new version available" check (spec section 14). Never run from a hook.

use tokred_core::config::UpdateMode;
use tokred_core::meta::MetaRepo;
use tokred_core::ports::Clock;
use tokred_core::release::ReleaseSource;

use crate::install::{InstallMethod, upgrade_command};
use crate::version::{is_newer, parse};

/// Meta key: when the latest release was last queried, successfully or not (Unix seconds).
pub const VERSION_CHECK_AT: &str = "version_check_at";
/// Meta key: the latest version seen at the last successful query (`0.4.0`).
pub const VERSION_CHECK_LATEST: &str = "version_check_latest";

/// Minimum time between queries.
const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// A published version newer than the running one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVersion {
    /// The version, without a leading `v` (`0.4.0`).
    pub version: String,
}

/// Whether to look for new versions at all: not when `[update] mode = "off"` and not when
/// `TOKRED_NO_UPDATE_CHECK` is set (`env_disabled`).
#[must_use]
pub fn should_check(mode: UpdateMode, env_disabled: bool) -> bool {
    mode != UpdateMode::Off && !env_disabled
}

/// Whether to replace the binary automatically: `auto` mode on a script install. With package
/// managers `auto` behaves like `notify`.
#[must_use]
pub fn should_self_update(mode: UpdateMode, method: InstallMethod) -> bool {
    mode == UpdateMode::Auto && method == InstallMethod::Script
}

/// Whether the latest release should be queried now: the last query is at least 24 hours old (or
/// missing, unreadable, or in the future). A broken meta store answers `false`, so a failing
/// database never causes a request on every command.
#[must_use]
pub fn check_due(meta: &dyn MetaRepo, clock: &dyn Clock) -> bool {
    let Ok(last) = meta.meta_get(VERSION_CHECK_AT) else {
        return false;
    };
    let last: Option<i64> = last.and_then(|value| value.trim().parse().ok());
    last.is_none_or(|last| !(0..CHECK_INTERVAL_SECS).contains(&(clock.now().0 - last)))
}

/// Runs the daily check and returns the newer version, if any.
///
/// When a query is due, the attempt time is recorded first (so a failure is retried tomorrow, not
/// on every command) and, on success, the latest version is cached in the meta store. The answer
/// always comes from the cache, compared with `current` at read time: within 24 hours no request
/// is made, and a user who just upgraded stops seeing the notice. Network errors are silent.
pub fn run_check(
    source: &dyn ReleaseSource,
    meta: &dyn MetaRepo,
    clock: &dyn Clock,
    current: &str,
) -> Option<NewVersion> {
    if check_due(meta, clock) {
        // Bookkeeping only: if a write fails we merely query again on the next run.
        let _ = meta.meta_set(VERSION_CHECK_AT, &clock.now().0.to_string());
        if let Ok(release) = source.latest()
            && let Some(latest) = parse(&release.version)
        {
            let _ = meta.meta_set(VERSION_CHECK_LATEST, &latest.to_string());
        }
    }
    let cached = meta.meta_get(VERSION_CHECK_LATEST).ok().flatten()?;
    is_newer(current, &cached).then(|| NewVersion {
        version: cached.trim().trim_start_matches(['v', 'V']).to_owned(),
    })
}

/// The one-line notice printed to stderr, for example
/// `new version 0.4.0 available (current 0.3.2) → run: brew upgrade tokred`.
/// Script installs update themselves, so for them the command is `tokred update`.
#[must_use]
pub fn notice(new: &NewVersion, current: &str, method: InstallMethod) -> String {
    let command = upgrade_command(method).unwrap_or("tokred update");
    format!(
        "new version {} available (current {current}) \u{2192} run: {command}",
        new.version
    )
}

#[cfg(test)]
mod tests;
