#!/usr/bin/env bash
# Regenerates crates/tokred-pricing/data/claude-prices.json (the price table embedded in the
# binary) from LiteLLM's public price file. Run it from anywhere; commit the result.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
url="https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json"
out="$root/crates/tokred-pricing/data/claude-prices.json"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl --fail --silent --show-error --location --max-time 60 "$url" -o "$work/litellm.json"
cargo run --quiet --manifest-path "$root/Cargo.toml" -p tokred-pricing --example snapshot -- \
  "$work/litellm.json" "BerriAI/litellm model_prices_and_context_window.json, $(date -u +%F)" \
  > "$work/claude-prices.json"
mv "$work/claude-prices.json" "$out"
echo "wrote $out ($(grep -c '"claude-' "$out") models)"
