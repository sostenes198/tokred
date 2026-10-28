//! Reversible compression of large tool results (spec 6.6 and 6.7) and the `tokred mcp` server
//! with the `tokred_retrieve` tool. No information is lost: originals are kept and retrievable.
//!
//! Placeholder: implemented in phase F11.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert_eq!(env!("CARGO_PKG_NAME"), "tokred-compress");
    }
}
