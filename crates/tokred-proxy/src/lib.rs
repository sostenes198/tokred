//! The local proxy (spec section 6): listens on loopback, forwards `/v1/messages` and every other
//! endpoint to the configured upstream with streaming, observes the responses to measure usage
//! and latency, and runs the hook pipeline defined in `tokred_core::proxy`.
//!
//! Placeholder: implemented in phase F8.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert_eq!(env!("CARGO_PKG_NAME"), "tokred-proxy");
    }
}
