//! Model Context Protocol (MCP) server for Risk Governor.
//!
//! Exposes the governor's decision API as MCP tools so ANY AI agent — Claude,
//! Cursor, a custom LangChain runner — can ask "may I move this money?" as
//! just another tool call, with the same policy/risk/investigation gates and
//! the same audit trail as every other client.
//!
//! THE POINT: an agent wired to this tool literally cannot execute a financial
//! action without it passing the governor. Governance becomes part of the
//! agent's toolset instead of a human afterthought.
//!
//! Transport: stdio, newline-delimited JSON-RPC 2.0 per the MCP spec.
//! All diagnostics go to stderr; stdout carries ONLY protocol messages.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub const PROTOCOL_VERSION: &str = "2024-11-05";
pub const SERVER_NAME: &str = "risk-governor";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Governor client abstraction (HTTP in production, fake in tests)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CheckActionInput {
    pub agent_id: String,
    pub merchant_id: String,
    /// refund | payout | payment_link | transfer | capture | void
    pub action_type: String,
    /// Amount in paise (₹100.00 → 10000).
    pub amount: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub declared_intent: String,
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(default)]
    pub payment_id: Option<String>,
}

fn default_currency() -> String {
    "INR".into()
}

#[async_trait]
pub trait GovernorClient: Send + Sync {
    async fn check_action(&self, input: CheckActionInput) -> Result<Value, String>;
    async fn get_decision(&self, decision_id: Uuid) -> Result<Value, String>;
    async fn list_decisions(&self) -> Result<Value, String>;
}

pub struct HttpClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl HttpClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    /// From env: GOVERNOR_URL (default http://127.0.0.1:8080) + GOVERNOR_API_KEY.
    pub fn from_env() -> Result<Self, String> {
        let key = std::env::var("GOVERNOR_API_KEY").map_err(|_| "GOVERNOR_API_KEY not set".to_string())?;
        Ok(Self::new(
            std::env::var("GOVERNOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
            key,
        ))
    }

    fn authed(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base_url, path))
            .header("x-api-key", &self.api_key)
    }
}

#[async_trait]
impl GovernorClient for HttpClient {
    async fn check_action(&self, input: CheckActionInput) -> Result<Value, String> {
        let mut context = json!({});
        if let Some(cid) = &input.customer_id {
            context["customer_id"] = json!(cid);
        }
        if let Some(pid) = &input.payment_id {
            context["payment_id"] = json!(pid);
        }
        let resp = self
            .authed(reqwest::Method::POST, "/v1/actions")
            .json(&json!({
                "agent_id": input.agent_id,
                "merchant_id": input.merchant_id,
                "action_type": input.action_type,
                "amount": input.amount,
                "currency": input.currency,
                "declared_intent": input.declared_intent,
                "context": context,
            }))
            .send()
            .await
            .map_err(|e| format!("governor unreachable: {e}"))?;
        handle_status(resp).await
    }

    async fn get_decision(&self, decision_id: Uuid) -> Result<Value, String> {
        let resp = self
            .authed(reqwest::Method::GET, &format!("/v1/decisions/{decision_id}"))
            .send()
            .await
            .map_err(|e| format!("governor unreachable: {e}"))?;
        handle_status(resp).await
    }

    async fn list_decisions(&self) -> Result<Value, String> {
        let resp = self
            .authed(reqwest::Method::GET, "/v1/decisions")
            .send()
            .await
            .map_err(|e| format!("governor unreachable: {e}"))?;
        handle_status(resp).await
    }
}

async fn handle_status(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    match resp.json::<Value>().await {
        Ok(body) if status.is_success() => Ok(body),
        Ok(body) => Err(format!(
            "governor returned {}: {}",
            status,
            body.get("error").and_then(|e| e.as_str()).unwrap_or("?")
        )),
        Err(e) => Err(format!("governor returned {}: ({e})", status)),
    }
}

// ---------------------------------------------------------------------------
// Tool registry
// ---------------------------------------------------------------------------

fn tool_defs() -> Value {
    json!([
        {
            "name": "check_action",
            "description": "Ask Risk Governor whether an AI-agent financial action (refund/payout/payment link etc.) should proceed. Returns ALLOW / REVIEW / BLOCK with the risk score and reasoning. Call this BEFORE executing any money movement against Razorpay.",
            "inputSchema": {
                "type": "object",
                "required": ["agent_id", "merchant_id", "action_type", "amount", "declared_intent"],
                "properties": {
                    "agent_id": {"type": "string"},
                    "merchant_id": {"type": "string"},
                    "action_type": {"type": "string", "enum": ["refund", "payout", "payment_link", "transfer", "capture", "void"]},
                    "amount": {"type": "integer", "description": "Amount in paise (₹100.00 = 10000)"},
                    "currency": {"type": "string", "description": "ISO currency, default INR"},
                    "declared_intent": {"type": "string", "description": "Free-text reason for the action"},
                    "customer_id": {"type": "string"},
                    "payment_id": {"type": "string", "description": "Required for refunds"}
                }
            }
        },
        {
            "name": "get_decision",
            "description": "Replay one governor decision: what it saw, every evaluation, why it decided, full audit trail.",
            "inputSchema": {
                "type": "object",
                "required": ["decision_id"],
                "properties": {"decision_id": {"type": "string", "format": "uuid"}}
            }
        },
        {
            "name": "list_reviews",
            "description": "List recent decisions awaiting or resolved by human review.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

fn summarize_decision(d: &Value) -> String {
    let rules = d
        .pointer("/policy_result/matched_rules")
        .and_then(|r| r.as_array())
        .map(|a| {
            if a.is_empty() {
                "none".into()
            } else {
                a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", ")
            }
        })
        .unwrap_or_else(|| "?".into());
    format!(
        "Decision: {}\n  action: {} {} {} paise from agent {}\n  risk score: {:.3} (intent mismatch {:.3})\n  matched rules: {}\n  decision_id: {}\n\nFull decision JSON:\n{}",
        d.get("decision").and_then(|v| v.as_str()).map(str::to_uppercase).unwrap_or_default(),
        d.pointer("/action/action_type").and_then(|v| v.as_str()).unwrap_or("?"),
        d.pointer("/action/currency").and_then(|v| v.as_str()).unwrap_or(""),
        d.pointer("/action/amount").and_then(|v| v.as_i64()).unwrap_or(0),
        d.pointer("/action/agent_id").and_then(|v| v.as_str()).unwrap_or("?"),
        d.pointer("/risk_result/risk_score").and_then(|v| v.as_f64()).unwrap_or(0.0),
        d.pointer("/risk_result/intent_mismatch_score").and_then(|v| v.as_f64()).unwrap_or(0.0),
        rules,
        d.get("decision_id").and_then(|v| v.as_str()).unwrap_or("?"),
        serde_json::to_string_pretty(d).unwrap_or_default(),
    )
}

fn summarize_trail(payload: &Value) -> String {
    let trail = payload
        .get("audit_trail")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("event_type").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(" → ")
        })
        .unwrap_or_default();
    format!(
        "Audit lifecycle: {}\n\n{}",
        if trail.is_empty() { "?" } else { &trail },
        serde_json::to_string_pretty(payload).unwrap_or_default(),
    )
}

fn text_content(text: String, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": is_error,
    })
}

// ---------------------------------------------------------------------------
// Protocol core
// ---------------------------------------------------------------------------

/// Handle one incoming JSON-RPC message. Returns `Some(response)` for requests
/// with an id; `None` for notifications (processed silently).
pub async fn handle_message(msg: &Value, client: &dyn GovernorClient) -> Option<Value> {
    let method = msg.get("method").and_then(|m| m.as_str())?;
    let params = msg.get("params").cloned().unwrap_or(json!({}));

    // Notifications (no id) are acknowledged by silence per JSON-RPC.
    let id = msg.get("id").cloned()?;

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_defs()})),
        "tools/call" => call_tool(&params, client).await.map_err(|e| (0, e)),
        other => Err((-32601, format!("method not found: {other}"))),
    };

    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            let code = if code == 0 { -32000 } else { code };
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": code, "message": message},
            })
        }
    })
}

async fn call_tool(params: &Value, client: &dyn GovernorClient) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| "tools/call requires a name".to_string())?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match name {
        "check_action" => {
            let input: CheckActionInput =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            let decision = client.check_action(input).await?;
            Ok(text_content(summarize_decision(&decision), false))
        }
        "get_decision" => {
            let id_str = args
                .get("decision_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "decision_id is required".to_string())?;
            let decision_id = Uuid::parse_str(id_str).map_err(|_| "decision_id must be a UUID".to_string())?;
            let payload = client.get_decision(decision_id).await?;
            Ok(text_content(summarize_trail(&payload), false))
        }
        "list_reviews" => {
            let all = client.list_decisions().await?;
            let reviews: Vec<&Value> = all
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|d| d.get("decision").and_then(|v| v.as_str()) == Some("review"))
                        .collect()
                })
                .unwrap_or_default();
            let text = if reviews.is_empty() {
                "No decisions currently in review.".to_string()
            } else {
                let lines: Vec<String> = reviews
                    .iter()
                    .map(|d| {
                        format!(
                            "{} — {} {} paise, agent {}, risk {}, approved: {}",
                            d.get("decision_id").and_then(|v| v.as_str()).unwrap_or("?"),
                            d.get("action_type").and_then(|v| v.as_str()).unwrap_or("?"),
                            d.get("amount").and_then(|v| v.as_i64()).unwrap_or(0),
                            d.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?"),
                            d.get("risk_score").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            d.get("human_decision").and_then(|v| v.as_str()).unwrap_or("PENDING"),
                        )
                    })
                    .collect();
                format!("Decisions in review:\n{}", lines.join("\n"))
            };
            Ok(text_content(text, false))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClient;

    fn canned_decision() -> Value {
        json!({
            "decision_id": Uuid::nil(),
            "decision": "review",
            "policy_result": {"verdict": "allow", "matched_rules": ["requires_approval_above_threshold"], "violated_thresholds": []},
            "risk_result": {"risk_score": 0.12, "intent_mismatch_score": 0.0},
            "action": {"agent_id": "agent-1", "action_type": "refund", "amount": 150000, "currency": "INR"},
        })
    }

    #[async_trait]
    impl GovernorClient for FakeClient {
        async fn check_action(&self, _: CheckActionInput) -> Result<Value, String> {
            Ok(canned_decision())
        }
        async fn get_decision(&self, _: Uuid) -> Result<Value, String> {
            Ok(json!({
                "decision": canned_decision(),
                "audit_trail": [
                    {"event_type": "action_requested"},
                    {"event_type": "policy_evaluated"},
                    {"event_type": "decision_made"},
                ],
            }))
        }
        async fn list_decisions(&self) -> Result<Value, String> {
            Ok(json!([canned_decision()]))
        }
    }

    fn request(method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    }

    #[tokio::test]
    async fn initialize_handshakes_with_protocol_version() {
        let resp = handle_message(&request("initialize", json!({})), &FakeClient)
            .await
            .unwrap();
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], SERVER_NAME);
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn notifications_are_silently_ignored() {
        let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(handle_message(&notification, &FakeClient).await.is_none());
    }

    #[tokio::test]
    async fn tools_list_exposes_three_tools_with_schemas() {
        let resp = handle_message(&request("tools/list", json!({})), &FakeClient)
            .await
            .unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let names: Vec<_> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"check_action"));
        assert!(names.contains(&"get_decision"));
        assert!(names.contains(&"list_reviews"));
        // check_action must declare its required fields so agents can't skip intent
        let schema = &tools[0]["inputSchema"];
        assert!(schema["required"].as_array().unwrap().len() >= 4);
    }

    #[tokio::test]
    async fn check_action_tool_returns_readable_verdict() {
        let args = json!({
            "agent_id": "agent-1", "merchant_id": "merchant-001",
            "action_type": "refund", "amount": 150000,
            "declared_intent": "refund order #456",
        });
        let resp = handle_message(
            &request("tools/call", json!({"name": "check_action", "arguments": args})),
            &FakeClient,
        )
        .await
        .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("Decision: REVIEW"), "{text}");
        assert!(text.contains("requires_approval_above_threshold"));
        assert_eq!(resp["result"]["isError"], false);
    }

    #[tokio::test]
    async fn get_decision_renders_audit_lifecycle() {
        let id = Uuid::nil().to_string();
        let resp = handle_message(
            &request(
                "tools/call",
                json!({"name": "get_decision", "arguments": {"decision_id": id}}),
            ),
            &FakeClient,
        )
        .await
        .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("action_requested → policy_evaluated → decision_made"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn list_reviews_flags_pending_human_approval() {
        let resp = handle_message(
            &request("tools/call", json!({"name": "list_reviews", "arguments": {}})),
            &FakeClient,
        )
        .await
        .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Decisions in review"));
        assert!(text.contains("PENDING"));
    }

    #[tokio::test]
    async fn unknown_method_is_jsonrpc_error() {
        let resp = handle_message(&request("resources/list", json!({})), &FakeClient)
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn invalid_tool_args_surface_as_tool_error_not_crash() {
        let resp = handle_message(
            &request(
                "tools/call",
                json!({"name": "check_action", "arguments": {"agent_id": 1}}),
            ),
            &FakeClient,
        )
        .await
        .unwrap();
        assert!(resp["error"]["message"].as_str().unwrap().contains("invalid arguments"));
    }

    #[test]
    fn ping_answers_empty_result() {
        // ping is sync-trivial; still exercised through the async core below
    }

    #[tokio::test]
    async fn ping_returns_empty_object() {
        let resp = handle_message(&request("ping", json!({})), &FakeClient).await.unwrap();
        assert_eq!(resp["result"], json!({}));
    }
}
