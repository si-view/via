use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `via send` → `via serve`: request to evaluate one SKILL expression.
#[derive(Debug, Serialize, Deserialize)]
pub struct EvalRequest {
    /// Client-generated UUID — used to correlate the response.
    pub id: String,
    /// Shared secret; must match server `--secret` (empty string = no auth).
    pub secret: String,
    /// SKILL expression to evaluate inside Virtuoso.
    pub expression: String,
    /// If true the server evaluates the expression but sends no response.
    #[serde(default)]
    pub no_reply: bool,
}

/// `via serve` → `via send`: result of evaluation.
#[derive(Debug, Serialize, Deserialize)]
pub struct EvalResponse {
    pub id: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Outcome {
    Success { result: Value },
    Failure { error: String },
}
