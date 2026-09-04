//! stdio loop: newline-delimited JSON-RPC in on stdin, responses on stdout.
//! Diagnostics go to stderr — stdout is protocol-only.
//!
//! Concurrency: each input line is handled in its own Tokio task (bounded by
//! MAX_INFLIGHT) so one slow governor call never head-of-line-blocks other
//! tool calls. Responses are matched to requests by JSON-RPC `id`, so
//! completion order need not match arrival order; a single writer task owns
//! stdout (via an mpsc channel) so frames never interleave.

use mcp_server::HttpClient;
use std::io::{BufRead, Write};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

/// Bound on concurrent in-flight tool calls — prevents task explosion if an
/// agent fans out aggressively; the 17th caller waits for a permit.
const MAX_INFLIGHT: usize = 16;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

    let client = Arc::new(HttpClient::from_env().map_err(anyhow::Error::msg)?);
    eprintln!(
        "risk-governor MCP server ready — governor at {}",
        std::env::var("GOVERNOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into())
    );

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Single writer task: the only code that touches stdout. The lock is
    // taken per message (never held across .await) so frames stay whole.
    let writer = tokio::spawn(async move {
        while let Some(resp) = rx.recv().await {
            let mut stdout = std::io::stdout().lock();
            if writeln!(stdout, "{resp}").is_err() {
                break;
            }
            if stdout.flush().is_err() {
                break;
            }
        }
    });

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let client = client.clone();
        let tx = tx.clone();
        let semaphore = semaphore.clone();
        tokio::spawn(async move {
            // Permit held for the whole handler — bounds concurrency.
            let _permit = semaphore.acquire_owned().await;
            let out = match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(msg) => mcp_server::handle_message(&msg, &*client).await,
                Err(e) => {
                    // Unparseable line: JSON-RPC parse error with null id.
                    Some(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32700, "message": format!("parse error: {e}")},
                    }))
                }
            };
            if let Some(resp) = out {
                let _ = tx.send(resp.to_string());
            }
        });
    }
    // stdin closed: wait for in-flight handlers, then close the channel so the
    // writer finishes instead of hanging forever.
    drop(tx);
    writer.await.ok();
    Ok(())
}
