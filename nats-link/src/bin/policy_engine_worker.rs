//! policy-engine as its own process: subscribes to NATS, evaluates, replies.
//!
//! Env:
//!   NATS_URL (default nats://127.0.0.1:4222)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    risk_governor_correlation::init_tracing("info");
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let client = async_nats::ConnectOptions::new()
        .connection_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await?;

    tracing::info!("policy-engine-worker starting");
    nats_link::spawn_policy_worker(client).await.ok();
    Ok(())
}
