use super::{Transport, TransportError};
use crate::types::{
    AgentStats, OperationStats, QueryFilter, UsageRecord, UsageSummary,
};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Persistent SQLite transport. Records survive process restarts and are queryable.
/// Uses WAL mode for concurrent reads.
///
/// Requires the `sqlite` feature flag.
#[derive(Clone)]
pub struct SqliteTransport {
    conn: Arc<Mutex<Connection>>,
    table: String,
}

impl SqliteTransport {
    /// Create or open a SQLite database at `path`. Use `:memory:` for in-memory.
    pub fn new(path: &str, table_name: Option<&str>) -> Result<Self, TransportError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        let table = table_name.unwrap_or("usage_records").to_string();

        conn.execute_batch(&format!(
            r#"
            CREATE TABLE IF NOT EXISTS {table} (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                service_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                agent_name TEXT,
                agent_shepherd_id TEXT,
                agent_tier TEXT,
                operation TEXT NOT NULL,
                units REAL NOT NULL DEFAULT 1,
                unit_type TEXT NOT NULL DEFAULT 'request',
                pricing_model TEXT NOT NULL DEFAULT 'per-call',
                method TEXT NOT NULL,
                path TEXT NOT NULL,
                status_code INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                request_signature TEXT,
                metadata TEXT
            );
            CREATE INDEX IF NOT EXISTS {table}_agent_id ON {table}(agent_id);
            CREATE INDEX IF NOT EXISTS {table}_service_id ON {table}(service_id);
            CREATE INDEX IF NOT EXISTS {table}_timestamp ON {table}(timestamp);
            "#,
            table = table
        ))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            table,
        })
    }

    fn insert_sync(&self, record: &UsageRecord) -> Result<(), TransportError> {
        let conn = self.conn.lock().unwrap();
        let metadata = record
            .metadata
            .as_ref()
            .map(|m| serde_json::to_string(m).ok())
            .flatten();

        conn.execute(
            &format!(
                r#"INSERT OR REPLACE INTO {table}
                (id, timestamp, service_id, agent_id, agent_name, agent_shepherd_id, agent_tier,
                 operation, units, unit_type, pricing_model, method, path, status_code,
                 duration_ms, request_signature, metadata)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)"#,
                table = self.table
            ),
            params![
                record.id,
                record.timestamp,
                record.service_id,
                record.agent.agent_id,
                record.agent.name,
                record.agent.shepherd_id,
                record.agent.tier,
                record.operation,
                record.units,
                record.unit_type,
                record.pricing_model.to_string(),
                record.method,
                record.path,
                record.status_code,
                record.duration_ms,
                record.request_signature,
                metadata,
            ],
        )?;
        Ok(())
    }

    /// Query records with optional filters.
    pub fn query(&self, filter: Option<&QueryFilter>) -> Result<Vec<UsageRecord>, TransportError> {
        let conn = self.conn.lock().unwrap();
        let (where_clause, values) = build_where(filter);
        let sql = format!(
            "SELECT * FROM {table} {where} ORDER BY timestamp DESC {limit} {offset}",
            table = self.table,
            where = where_clause,
            limit = filter
                .and_then(|f| f.limit)
                .map(|l| format!("LIMIT {}", l))
                .unwrap_or_default(),
            offset = filter
                .and_then(|f| f.offset)
                .map(|o| format!("OFFSET {}", o))
                .unwrap_or_default(),
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), row_to_record)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    /// Count records matching filter.
    pub fn count(&self, filter: Option<&QueryFilter>) -> Result<usize, TransportError> {
        let conn = self.conn.lock().unwrap();
        let (where_clause, values) = build_where(filter);
        let sql = format!(
            "SELECT COUNT(*) FROM {table} {where}",
            table = self.table,
            where = where_clause
        );
        let count: i64 = conn.query_row(
            &sql,
            rusqlite::params_from_iter(values.iter()),
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Summarize usage.
    pub fn summary(&self, filter: Option<&QueryFilter>) -> Result<UsageSummary, TransportError> {
        let records = self.query(filter)?;
        let mut summary = UsageSummary::default();
        let mut agents = std::collections::HashSet::new();

        for record in &records {
            summary.total_records += 1;
            summary.total_units += record.units;
            agents.insert(record.agent.agent_id.clone());

            let op = summary
                .by_operation
                .entry(record.operation.clone())
                .or_insert_with(OperationStats::default);
            op.count += 1;
            op.units += record.units;

            let ag = summary
                .by_agent
                .entry(record.agent.agent_id.clone())
                .or_insert_with(AgentStats::default);
            ag.count += 1;
            ag.units += record.units;
        }

        summary.unique_agents = agents.len();
        Ok(summary)
    }

    /// Close the database connection cleanly.
    pub fn close(self) -> Result<(), TransportError> {
        // Connection closes on drop. Nothing special needed.
        Ok(())
    }
}

fn build_where(filter: Option<&QueryFilter>) -> (String, Vec<String>) {
    let Some(f) = filter else {
        return (String::new(), vec![]);
    };

    let mut clauses = Vec::new();
    let mut values: Vec<String> = Vec::new();

    if let Some(ref v) = f.agent_id {
        clauses.push("agent_id = ?".to_string());
        values.push(v.clone());
    }
    if let Some(ref v) = f.service_id {
        clauses.push("service_id = ?".to_string());
        values.push(v.clone());
    }
    if let Some(ref v) = f.operation {
        clauses.push("operation = ?".to_string());
        values.push(v.clone());
    }
    if let Some(ref v) = f.from {
        clauses.push("timestamp >= ?".to_string());
        values.push(v.clone());
    }
    if let Some(ref v) = f.to {
        clauses.push("timestamp <= ?".to_string());
        values.push(v.clone());
    }

    if clauses.is_empty() {
        (String::new(), vec![])
    } else {
        (format!("WHERE {}", clauses.join(" AND ")), values)
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecord> {
    use crate::types::{AgentIdentity, PricingModel};

    let pricing_str: String = row.get("pricing_model")?;
    let pricing_model = match pricing_str.as_str() {
        "per-call" => PricingModel::PerCall,
        "per-unit" => PricingModel::PerUnit,
        "per-minute" => PricingModel::PerMinute,
        "tiered" => PricingModel::Tiered,
        _ => PricingModel::Custom,
    };

    let metadata_str: Option<String> = row.get("metadata")?;
    let metadata = metadata_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(UsageRecord {
        id: row.get("id")?,
        timestamp: row.get("timestamp")?,
        service_id: row.get("service_id")?,
        agent: AgentIdentity {
            agent_id: row.get("agent_id")?,
            name: row.get("agent_name")?,
            shepherd_id: row.get("agent_shepherd_id")?,
            tier: row.get("agent_tier")?,
        },
        operation: row.get("operation")?,
        units: row.get("units")?,
        unit_type: row.get("unit_type")?,
        pricing_model,
        method: row.get("method")?,
        path: row.get("path")?,
        status_code: row.get::<_, i64>("status_code")? as u16,
        duration_ms: row.get::<_, i64>("duration_ms")? as u64,
        request_signature: row.get("request_signature")?,
        metadata,
    })
}

impl Transport for SqliteTransport {
    fn send(
        &self,
        record: UsageRecord,
    ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + '_>> {
        let result = self.insert_sync(&record);
        Box::pin(async move { result })
    }
}
