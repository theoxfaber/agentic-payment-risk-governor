//! Postgres-backed stores implementing the existing `AuditStore` and
//! `EvidenceStore` traits, plus decision persistence for replay/review.
//!
//! Schema is JSONB-first: payloads are stored exactly as the in-memory
//! stores hold them, so the wire format never diverges between backends.
//! Migrations run on connect — no external migration step.

use async_trait::async_trait;
use audit_service::{AuditError, AuditStore};
use evidence_service::{EvidenceError, EvidenceStore};
use risk_governor_types::*;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::str::FromStr;
use uuid::Uuid;

pub struct PgStore {
    pool: PgPool,
}
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS audit_records (
        record_id     UUID PRIMARY KEY,
        decision_id   UUID,
        event_type    TEXT NOT NULL,
        payload       JSONB NOT NULL,
        created_at    TIMESTAMPTZ NOT NULL,
        previous_hash TEXT,
        current_hash  TEXT NOT NULL DEFAULT ''
    )",
    "CREATE INDEX IF NOT EXISTS idx_audit_decision ON audit_records(decision_id)",
    "CREATE TABLE IF NOT EXISTS decisions (
        decision_id UUID PRIMARY KEY,
        outcome     TEXT NOT NULL,
        data        JSONB NOT NULL,
        created_at  TIMESTAMPTZ NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS evidence_agents (
        agent_id TEXT PRIMARY KEY,
        data     JSONB NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS evidence_merchants (
        merchant_id TEXT PRIMARY KEY,
        data        JSONB NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS evidence_customers (
        customer_id TEXT PRIMARY KEY,
        data        JSONB NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS action_log (
        agent_id TEXT NOT NULL,
        ts       TIMESTAMPTZ NOT NULL,
        amount   BIGINT NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_action_log_agent_ts ON action_log(agent_id, ts)",
];

impl PgStore {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new().max_connections(10).connect(url).await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        for stmt in SCHEMA {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        tracing::info!("pg-store schema ready ({} objects ensured)", SCHEMA.len());
        Ok(())
    }

    /// Idempotent seed of demo/reference entities from a JSON file with shape
    /// {"agents":[...],"merchants":[...],"customers":[...]}. Existing rows win.
    pub async fn seed_from_json(&self, raw: &str) -> anyhow::Result<(usize, usize, usize)> {
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct Seed {
            agents: Vec<serde_json::Value>,
            merchants: Vec<serde_json::Value>,
            customers: Vec<serde_json::Value>,
        }
        let seed: Seed = serde_json::from_str(raw)?;
        let mut counts = (0, 0, 0);
        for a in &seed.agents {
            let id = a["agent_id"].as_str().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }
            sqlx::query(
                "INSERT INTO evidence_agents (agent_id, data) VALUES ($1, $2) ON CONFLICT (agent_id) DO NOTHING",
            )
            .bind(&id)
            .bind(serde_json::Value::Object(a.as_object().cloned().unwrap_or_default()))
            .execute(&self.pool)
            .await?;
            counts.0 += 1;
        }
        for m in &seed.merchants {
            let id = m["merchant_id"].as_str().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }
            sqlx::query("INSERT INTO evidence_merchants (merchant_id, data) VALUES ($1, $2) ON CONFLICT (merchant_id) DO NOTHING")
                .bind(&id)
                .bind(serde_json::Value::Object(m.as_object().cloned().unwrap_or_default()))
                .execute(&self.pool)
                .await?;
            counts.1 += 1;
        }
        for c in &seed.customers {
            let id = c["customer_id"].as_str().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }
            sqlx::query("INSERT INTO evidence_customers (customer_id, data) VALUES ($1, $2) ON CONFLICT (customer_id) DO NOTHING")
                .bind(&id)
                .bind(serde_json::Value::Object(c.as_object().cloned().unwrap_or_default()))
                .execute(&self.pool)
                .await?;
            counts.2 += 1;
        }
        tracing::info!(
            agents = counts.0,
            merchants = counts.1,
            customers = counts.2,
            "seed applied"
        );
        Ok(counts)
    }

    // -- decisions ---------------------------------------------------------

    pub async fn upsert_decision(&self, d: &Decision) -> Result<(), AuditError> {
        sqlx::query(
            "INSERT INTO decisions (decision_id, outcome, data, created_at) VALUES ($1, $2, $3, $4)
             ON CONFLICT (decision_id) DO UPDATE SET outcome = EXCLUDED.outcome, data = EXCLUDED.data",
        )
        .bind(d.decision_id)
        .bind(serde_json::to_string(&d.decision).map_err(|e| AuditError::Write(e.to_string()))?)
        .bind(serde_json::to_value(d).map_err(|e| AuditError::Write(e.to_string()))?)
        .bind(d.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AuditError::Write(e.to_string()))?;
        Ok(())
    }

    pub async fn all_decisions(&self) -> Result<Vec<Decision>, AuditError> {
        let rows = sqlx::query_as::<_, (serde_json::Value,)>("SELECT data FROM decisions ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AuditError::Write(e.to_string()))?;
        rows.into_iter()
            .map(|(v,)| serde_json::from_value(v).map_err(|e| AuditError::Write(e.to_string())))
            .collect()
    }

    pub async fn get_decision(&self, id: Uuid) -> Result<Option<Decision>, AuditError> {
        let row = sqlx::query_as::<_, (serde_json::Value,)>("SELECT data FROM decisions WHERE decision_id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AuditError::Write(e.to_string()))?;
        match row {
            Some((v,)) => serde_json::from_value(v)
                .map(Some)
                .map_err(|e| AuditError::Write(e.to_string())),
            None => Ok(None),
        }
    }

    pub async fn counts_by_outcome(&self) -> Result<std::collections::HashMap<String, i64>, AuditError> {
        let rows = sqlx::query_as::<_, (String, i64)>("SELECT outcome, COUNT(*) FROM decisions GROUP BY outcome")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AuditError::Write(e.to_string()))?;
        Ok(rows.into_iter().collect())
    }
}

#[async_trait]
impl AuditStore for PgStore {
    async fn append(&self, mut record: AuditRecord) -> Result<(), AuditError> {
        if record.current_hash.is_empty() {
            let mut tx = self.pool.begin().await.map_err(|e| AuditError::Write(e.to_string()))?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(0xA941_i64)
                .execute(&mut *tx)
                .await
                .map_err(|e| AuditError::Write(e.to_string()))?;
            let last_hash: Option<String> = sqlx::query_as::<_, (Option<String>,)>(
                "SELECT current_hash FROM audit_records ORDER BY created_at DESC LIMIT 1",
            )
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| AuditError::Write(e.to_string()))?
            .and_then(|(h,)| h);

            record.previous_hash = last_hash;
            record.current_hash = AuditRecord::compute_hash(
                record.record_id,
                record.decision_id,
                record.event_type,
                &record.payload,
                record.created_at,
                record.previous_hash.as_deref(),
            );
            sqlx::query("INSERT INTO audit_records (record_id, decision_id, event_type, payload, created_at, previous_hash, current_hash) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                .bind(record.record_id)
                .bind(record.decision_id)
                .bind(event_type_str(record.event_type))
                .bind(record.payload.clone())
                .bind(record.created_at)
                .bind(record.previous_hash.clone())
                .bind(record.current_hash.clone())
                .execute(&mut *tx)
                .await
                .map_err(|e| AuditError::Write(e.to_string()))?;
            tx.commit().await.map_err(|e| AuditError::Write(e.to_string()))?;
            return Ok(());
        }

        sqlx::query("INSERT INTO audit_records (record_id, decision_id, event_type, payload, created_at, previous_hash, current_hash) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(record.record_id)
            .bind(record.decision_id)
            .bind(event_type_str(record.event_type))
            .bind(record.payload)
            .bind(record.created_at)
            .bind(record.previous_hash)
            .bind(record.current_hash)
            .execute(&self.pool)
            .await
            .map_err(|e| AuditError::Write(e.to_string()))?;
        Ok(())
    }

    async fn by_decision(&self, decision_id: Uuid) -> Result<Vec<AuditRecord>, AuditError> {
        let rows = sqlx::query_as::<_, (Uuid, Option<Uuid>, String, serde_json::Value, chrono::DateTime<chrono::Utc>, Option<String>, String)>(
            "SELECT record_id, decision_id, event_type, payload, created_at, previous_hash, current_hash FROM audit_records WHERE decision_id = $1 ORDER BY created_at ASC",
        )
        .bind(decision_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuditError::Write(e.to_string()))?;
        rows.into_iter().map(row_to_record).collect()
    }

    async fn all(&self) -> Result<Vec<AuditRecord>, AuditError> {
        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                Option<Uuid>,
                String,
                serde_json::Value,
                chrono::DateTime<chrono::Utc>,
                Option<String>,
                String,
            ),
        >(
            "SELECT record_id, decision_id, event_type, payload, created_at, previous_hash, current_hash FROM audit_records ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuditError::Write(e.to_string()))?;
        rows.into_iter().map(row_to_record).collect()
    }
}

fn event_type_str(t: AuditEventType) -> &'static str {
    match t {
        AuditEventType::ActionRequested => "action_requested",
        AuditEventType::PolicyEvaluated => "policy_evaluated",
        AuditEventType::RiskScored => "risk_scored",
        AuditEventType::GraphAnalyzed => "graph_analyzed",
        AuditEventType::DecisionMade => "decision_made",
        AuditEventType::HumanReviewed => "human_reviewed",
        AuditEventType::RazorpayCalled => "razorpay_called",
        AuditEventType::WebhookReceived => "webhook_received",
        AuditEventType::OutcomeRecorded => "outcome_recorded",
    }
}

fn parse_event_type(s: &str) -> Option<AuditEventType> {
    Some(match s {
        "action_requested" => AuditEventType::ActionRequested,
        "policy_evaluated" => AuditEventType::PolicyEvaluated,
        "risk_scored" => AuditEventType::RiskScored,
        "graph_analyzed" => AuditEventType::GraphAnalyzed,
        "decision_made" => AuditEventType::DecisionMade,
        "human_reviewed" => AuditEventType::HumanReviewed,
        "razorpay_called" => AuditEventType::RazorpayCalled,
        "webhook_received" => AuditEventType::WebhookReceived,
        "outcome_recorded" => AuditEventType::OutcomeRecorded,
        _ => return None,
    })
}

type RecordRow = (
    Uuid,
    Option<Uuid>,
    String,
    serde_json::Value,
    chrono::DateTime<chrono::Utc>,
    Option<String>,
    String,
);

fn row_to_record(
    (record_id, decision_id, event_type, payload, created_at, previous_hash, current_hash): RecordRow,
) -> Result<AuditRecord, AuditError> {
    Ok(AuditRecord {
        record_id,
        decision_id,
        event_type: parse_event_type(&event_type)
            .ok_or_else(|| AuditError::Write(format!("unknown event_type {event_type}")))?,
        payload,
        created_at,
        previous_hash,
        current_hash,
    })
}

#[async_trait]
impl EvidenceStore for PgStore {
    async fn agent_history(&self, agent_id: &str) -> Result<Option<AgentHistory>, EvidenceError> {
        let row = sqlx::query_as::<_, (serde_json::Value,)>("SELECT data FROM evidence_agents WHERE agent_id = $1")
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| EvidenceError::Storage(e.to_string()))?;
        match row {
            Some((v,)) => serde_json::from_value(v)
                .map(Some)
                .map_err(|e| EvidenceError::Storage(e.to_string())),
            None => Ok(None),
        }
    }

    async fn merchant_policy(&self, merchant_id: &str) -> Result<Option<MerchantPolicy>, EvidenceError> {
        let row =
            sqlx::query_as::<_, (serde_json::Value,)>("SELECT data FROM evidence_merchants WHERE merchant_id = $1")
                .bind(merchant_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| EvidenceError::Storage(e.to_string()))?;
        match row {
            Some((v,)) => serde_json::from_value(v)
                .map(Some)
                .map_err(|e| EvidenceError::Storage(e.to_string())),
            None => Ok(None),
        }
    }

    async fn customer_history(&self, customer_id: &str) -> Result<Option<CustomerHistory>, EvidenceError> {
        let row =
            sqlx::query_as::<_, (serde_json::Value,)>("SELECT data FROM evidence_customers WHERE customer_id = $1")
                .bind(customer_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| EvidenceError::Storage(e.to_string()))?;
        match row {
            Some((v,)) => serde_json::from_value(v)
                .map(Some)
                .map_err(|e| EvidenceError::Storage(e.to_string())),
            None => Ok(None),
        }
    }

    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), EvidenceError> {
        sqlx::query("INSERT INTO action_log (agent_id, ts, amount) VALUES ($1, $2, $3)")
            .bind(&request.agent_id)
            .bind(request.timestamp)
            .bind(request.amount)
            .execute(&self.pool)
            .await
            .map_err(|e| EvidenceError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Windowed velocity computed by the database — same semantics as the
    /// in-memory scan, but survives restarts and works across processes.
    async fn velocity(&self, agent_id: &str) -> Result<VelocityStats, EvidenceError> {
        let row = sqlx::query_as::<_, (i64, i64, i64, i64)>(
            r#"
            SELECT
                COALESCE(COUNT(*) FILTER (WHERE ts >= now() - interval '1 hour'), 0)::BIGINT,
                COALESCE(SUM(amount) FILTER (WHERE ts >= now() - interval '1 hour'), 0)::BIGINT,
                COALESCE(COUNT(*), 0)::BIGINT,
                COALESCE(SUM(amount), 0)::BIGINT
            FROM action_log
            WHERE agent_id = $1 AND ts >= now() - interval '24 hours'
            "#,
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| EvidenceError::Storage(e.to_string()))?;

        // Unique merchants/customers are not captured on action_log rows
        // today; the in-memory store also reports zeros for these.
        Ok(VelocityStats {
            actions_last_hour: row.0 as u32,
            volume_last_hour: row.1,
            actions_last_24h: row.2 as u32,
            volume_last_24h: row.3,
            unique_merchants_24h: 0,
            unique_customers_24h: 0,
        })
    }
}

/// Convenience: build a connection URL from individual env parts.
pub fn url_from_parts(user: &str, password: &str, host: &str, port: u16, db: &str) -> String {
    format!("postgres://{user}:{password}@{host}:{port}/{db}")
}

/// Parse + validate without connecting.
pub fn validate_url(url: &str) -> bool {
    sqlx::postgres::PgConnectOptions::from_str(url).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_roundtrip_covers_all_variants() {
        let all = [
            AuditEventType::ActionRequested,
            AuditEventType::PolicyEvaluated,
            AuditEventType::RiskScored,
            AuditEventType::GraphAnalyzed,
            AuditEventType::DecisionMade,
            AuditEventType::HumanReviewed,
            AuditEventType::RazorpayCalled,
            AuditEventType::WebhookReceived,
            AuditEventType::OutcomeRecorded,
        ];
        for t in all {
            assert_eq!(parse_event_type(event_type_str(t)), Some(t));
        }
        assert_eq!(parse_event_type("bogus"), None);
    }

    #[test]
    fn url_parts_roundtrip() {
        assert!(validate_url(&url_from_parts(
            "governor",
            "pw",
            "localhost",
            5432,
            "governor"
        )));
        assert!(!validate_url("not a url"));
    }
}
