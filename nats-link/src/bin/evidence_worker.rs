//! evidence-service as its own process. Serves gather + record_action.
//!
//! Env:
//!   NATS_URL      (default nats://127.0.0.1:4222)
//!   EVIDENCE_SEED optional JSON: {"agents":[...],"merchants":[...],"customers":[...]}
//!                 Interim until the Postgres-backed store lands.

use risk_governor_types::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    risk_governor_correlation::init_tracing("info");
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let client = async_nats::ConnectOptions::new()
        .connection_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await?;

    let store = std::sync::Arc::new(load_seed().await?);
    tracing::info!("evidence-worker starting");
    nats_link::spawn_evidence_worker(client, store).await.ok();
    Ok(())
}

async fn load_seed() -> anyhow::Result<evidence_service::InMemoryEvidenceStore> {
    use evidence_service::InMemoryEvidenceStore;
    let store = InMemoryEvidenceStore::new();

    match std::env::var("EVIDENCE_SEED") {
        Err(_) => {
            tracing::warn!("EVIDENCE_SEED not set — worker starts with an EMPTY store; every gather will be NotFound");
            Ok(store)
        }
        Ok(path) => {
            let raw = std::fs::read_to_string(&path)?;
            #[derive(serde::Deserialize, Default)]
            #[serde(default)]
            struct Seed {
                agents: Vec<AgentHistory>,
                merchants: Vec<MerchantPolicy>,
                customers: Vec<CustomerHistory>,
            }
            let seed: Seed = serde_json::from_str(&raw)?;

            for a in seed.agents {
                store.seed_agent(a).await;
            }
            for m in seed.merchants {
                store.seed_merchant_policy(m).await;
            }
            for c in seed.customers {
                store.seed_customer(c).await;
            }
            tracing::info!(path = %path, "evidence seed loaded");
            Ok(store)
        }
    }
}