//! evidence-service as its own process. Serves gather + record_action.
//!
//! Env:
//!   NATS_URL      (default nats://127.0.0.1:4222)
//!   DATABASE_URL  set → Postgres-backed store (survives restarts, shared
//!                 across processes); unset → in-memory with optional seed.
//!   EVIDENCE_SEED optional JSON: {"agents":[...],"merchants":[...],"customers":[...]}
//!                 Applied to either backend (PG: idempotent, existing rows win).

use risk_governor_types::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    risk_governor_correlation::init_tracing("info");
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let client = async_nats::ConnectOptions::new()
        .connection_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await?;

    let seed_raw = match std::env::var("EVIDENCE_SEED") {
        Ok(path) if !path.is_empty() => Some(std::fs::read_to_string(&path)?),
        _ => {
            tracing::warn!("EVIDENCE_SEED not set — worker may start without reference entities");
            None
        }
    };

    match std::env::var("DATABASE_URL") {
        Ok(db) if !db.is_empty() => {
            let store = Arc::new(pg_store::PgStore::connect(&db).await?);
            if let Some(raw) = &seed_raw {
                store.seed_from_json(raw).await?;
            }
            tracing::info!("evidence-worker starting (Postgres-backed)");
            nats_link::spawn_evidence_worker(client, store).await.ok();
        }
        _ => {
            let store = Arc::new(build_mem_store(seed_raw.as_deref()).await?);
            tracing::info!("evidence-worker starting (in-memory)");
            nats_link::spawn_evidence_worker(client, store).await.ok();
        }
    }
    Ok(())
}

/// Build an InMemoryEvidenceStore from the optional seed file.
///
/// Note on the historical nested-runtime panic (BUGS.md-class bug): seeding
/// happens *before* any runtime nesting and all store methods are awaited
/// directly — never `block_on` inside the tokio runtime.
async fn build_mem_store(seed_raw: Option<&str>) -> anyhow::Result<evidence_service::InMemoryEvidenceStore> {
    use evidence_service::InMemoryEvidenceStore;
    let store = InMemoryEvidenceStore::new();

    let Some(raw) = seed_raw else {
        return Ok(store);
    };

    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    struct Seed {
        agents: Vec<AgentHistory>,
        merchants: Vec<MerchantPolicy>,
        customers: Vec<CustomerHistory>,
    }
    let seed: Seed = serde_json::from_str(raw)?;

    for a in seed.agents {
        store.seed_agent(a).await;
    }
    for m in seed.merchants {
        store.seed_merchant_policy(m).await;
    }
    for c in seed.customers {
        store.seed_customer(c).await;
    }
    Ok(store)
}
