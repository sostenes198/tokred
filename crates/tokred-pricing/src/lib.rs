//! Price table, sync and embedded fallback for tokred.
//!
//! * [`parse_litellm`] turns `LiteLLM`'s public price file into [`tokred_core::pricing::PriceEntry`]
//!   rows (Claude models only, USD per million tokens).
//! * [`fallback_entries`] is the snapshot embedded in the binary, used when there is no network.
//! * [`http::LiteLlmSource`] is the network adapter of the [`tokred_core::pricing::PriceSource`]
//!   port.
//! * [`sync_prices`], [`load_table`] and [`sync_due`] are the use cases; every port is a parameter.

mod error;
mod litellm;
mod sync;

pub mod http;

pub use error::PriceError;
pub use litellm::{
    EMBEDDED_SOURCE, LITELLM_SOURCE, fallback_entries, parse_litellm, render_snapshot,
};
pub use sync::{
    PRICE_SYNC_ATTEMPT_AT, PRICE_SYNCED_AT, SyncOutcome, load_table, sync_due, sync_prices,
};
