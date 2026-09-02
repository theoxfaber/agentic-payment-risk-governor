//! stdio loop: newline-delimited JSON-RPC in on stdin, responses on stdout.
//! Diagnostics go to stderr — stdout is protocol-only.

use mcp_server::HttpClient;
use std::io::{BufRead, Write};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

    let client = HttpClient::from_env().map_err(anyhow::Error::msg)?;
    eprintln!(
        "risk-governor MCP server ready — governor at {}",
        std::env::var("GOVERNOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into())
    );

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(msg) => {
                // Sequential handling keeps tool-call ordering deterministic.
                if let Some(resp) = mcp_server::handle_message(&msg, &client).await {
                    writeln!(stdout, "{resp}")?;
                    stdout.flush()?;
                }
            }
            Err(e) => {
                // Unparseable line: JSON-RPC parse error with null id.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")},
                });
                writeln!(stdout, "{resp}")?;
                stdout.flush()?;
            }
        }
    }
    Ok(())
}
