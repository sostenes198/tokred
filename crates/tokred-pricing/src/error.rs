//! Errors of the price parser.

/// The price file could not be turned into price rows.
#[derive(Debug, thiserror::Error)]
pub enum PriceError {
    /// The text is not valid JSON.
    #[error("price file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The JSON is valid, but its top level is not an object of models.
    #[error("price file must be a JSON object keyed by model")]
    NotAnObject,
    /// No usable Claude model was found (an empty result must never replace a good table).
    #[error("price file contains no usable Claude model")]
    NoClaudeModels,
}
