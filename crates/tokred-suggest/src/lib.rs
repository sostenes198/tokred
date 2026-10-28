//! Summaries of the main loop's prompts and suggestions of a cheaper model (spec 5.6). The main
//! loop's model is never changed: tokred only advises.
//!
//! Placeholder: implemented in phase F10.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert_eq!(env!("CARGO_PKG_NAME"), "tokred-suggest");
    }
}
