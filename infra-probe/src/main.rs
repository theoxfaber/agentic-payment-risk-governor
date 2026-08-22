//! Throwaway connectivity probe: proves NATS pub/sub loopback and Postgres
//! SELECT 1 work from Rust BEFORE any real service touches the infra.
//! Delete after Phase 2 wiring is stable.

use futures::StreamExt;
use std::time::Duration;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    probe_nats().await?;
    probe_postgres().await?;
    println!("infra-probe: ALL OK");
    Ok(())
}

async fn connect_nats(url: &str) -> anyhow::Result<async_nats::Client> {
    let mut last_err = None;
    for attempt in 1..=5 {
        match async_nats::ConnectOptions::new()
            .connection_timeout(Duration::from_secs(3))
            .connect(url)
            .await
        {
            Ok(client) => return Ok(client),
            Err(e) => {
                eprintln!("nats: connect attempt {attempt} failed: {e}");
                last_err = Some(e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    Err(anyhow::anyhow!(
        "nats: could not connect after 5 attempts: {}",
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ))
}

async fn probe_nats() -> anyhow::Result<()> {
    let client = connect_nats("nats://127.0.0.1:4222").await?;

    // Core-NATS round trip: subscribe, publish, receive own message.
    let subject = format!("probe.{}", Uuid::new_v4());
    let mut sub = client.subscribe(subject.clone()).await?;

    let sent = b"ping".to_vec();
    client.publish(subject.clone(), sent.clone().into()).await?;
    client.flush().await?;

    let msg = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .map_err(|_| anyhow::anyhow!("nats: timed out waiting for own message"))?
        .ok_or_else(|| anyhow::anyhow!("nats: subscription closed"))?;

    assert_eq!(msg.payload.as_ref(), sent.as_slice());
    println!("nats: OK (pub/sub round trip on {subject})");
    Ok(())
}

async fn probe_postgres() -> anyhow::Result<()> {
    let url = "postgres://governor:governor@127.0.0.1:5432/governor";
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await?;

    let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await?;
    assert_eq!(row.0, 1);
    println!("postgres: OK (SELECT 1)");
    Ok(())
}