use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `via send` → `via serve`: request to evaluate one SKILL expression.
#[derive(Debug, Serialize, Deserialize)]
pub struct EvalRequest {
    /// Client-generated UUID — used to correlate the response.
    pub id: String,
    /// SKILL expression to evaluate inside Virtuoso.
    pub expression: String,
    /// If true the server evaluates the expression but sends no response.
    #[serde(default)]
    pub no_reply: bool,
}

/// Unified result payload — carried in both the `S:` callback line and the
/// final `EvalResponse` sent back to `via send`.
///
/// | field   | meaning                                                        |
/// |---------|----------------------------------------------------------------|
/// | data    | JSON-serialised SKILL value; `null` on failure                 |
/// | reason  | error description; `Some` → failure, `None` → success         |
/// | is_ref  | `true` when `data` is a remote-object stub `{id, kind}`       |
/// | code    | reserved for future result subtypes (0 = normal result)       |
#[derive(Debug, Serialize, Deserialize)]
pub struct EvalResult {
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub is_ref: bool,
    pub code: i64,
}

/// `via serve` → `via send`: response envelope.
///
/// Success: `{"id":"…","ok":true, "data":…,"is_ref":false,"code":0}`
/// Failure: `{"id":"…","ok":false,"data":null,"reason":"…","is_ref":false,"code":0}`
#[derive(Debug, Serialize, Deserialize)]
pub struct EvalResponse {
    pub id: String,
    pub ok: bool,
    #[serde(flatten)]
    pub result: EvalResult,
}

impl EvalResponse {
    pub fn success(id: String, result: EvalResult) -> Self {
        Self {
            id,
            ok: true,
            result,
        }
    }
    pub fn failure(id: String, reason: String) -> Self {
        Self {
            id,
            ok: false,
            result: EvalResult {
                data: Value::Null,
                reason: Some(reason),
                is_ref: false,
                code: 0,
            },
        }
    }
}
