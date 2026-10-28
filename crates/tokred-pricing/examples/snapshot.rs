//! Regenerates `data/claude-prices.json` from a downloaded `LiteLLM` price file.
//!
//! Usage: `cargo run -p tokred-pricing --example snapshot -- <litellm.json> <generated note>`
//! (see `scripts/update-price-snapshot.sh`). The snapshot is written to stdout.

use std::io::Write;

use tokred_core::types::Timestamp;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: snapshot <litellm.json> [note]")?;
    let note = args.next().unwrap_or_default();
    let json = std::fs::read_to_string(path)?;
    let entries = tokred_pricing::parse_litellm(&json, Timestamp(0))?;
    let text = tokred_pricing::render_snapshot(&entries, &note);
    std::io::stdout().write_all(text.as_bytes())?;
    Ok(())
}
