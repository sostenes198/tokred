//! Hook handlers invoked by Claude Code as `tokred hook <event>`.
//!
//! Every handler is **fail-open**: on error, panic, bad input or timeout the hook prints
//! nothing and the caller exits with code 0, so a tokred bug can never block Claude Code.
//!
//! The handlers are pure over the ports in [`ports`] (repositories from `tokred-core`, a
//! [`ports::FileMeta`] and a clock); only [`fs`] touches the real machine, and the `tokred`
//! binary is the single place that wires the adapters together. [`run_hook`] is the runtime:
//! it reads stdin, runs the handler, writes the answer and then records the execution.
//!
//! Output formats follow <https://code.claude.com/docs/en/hooks>; see `docs/hooks-and-agents.md`.

pub mod bench;
pub mod classify;
mod error;
pub mod fs;
mod guard;
mod io;
pub mod ports;
mod route;
mod runtime;
mod savings_hint;
mod session;
mod suggest;
pub mod watchdog;

pub use bench::bench_fixtures;
pub use classify::{Rules, classify};
pub use error::HookError;
pub use guard::MAX_HASH_BYTES;
pub use io::HookInput;
pub use ports::{EventNote, FileMeta, FileStat, HookDeps, HookResponse};
pub use runtime::{
    HookRun, MAX_STDIN_BYTES, handle, handle_with, record_timeout, run_hook, run_hook_with,
};
pub use session::last_context_tokens;
pub use tokred_core::cache::{cache_prune, cache_stats};
