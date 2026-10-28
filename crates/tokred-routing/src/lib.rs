//! Model routing policy (spec section 5): classifies subagent and auxiliary requests and picks a
//! cheaper model when it is safe and worth it. The main loop's model is never changed.
//!
//! Placeholder: implemented in phase F9.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert_eq!(env!("CARGO_PKG_NAME"), "tokred-routing");
    }
}
