use std::{
    io::BufReader,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use rusqlite::{Connection, OpenFlags, Transaction, params, params_from_iter};
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;

type RawEvent = (f64, f64, i64, i64, i64, i64, i64, i64, i64, String);
const UI_SCHEMA_VERSION: i64 = 11;
const EVENT_INSERT_BATCH: usize = 64;
const TIMELINE_MIPMAP_WIDTH_SECONDS: f64 = 0.001;
const TIMELINE_MIPMAP_COARSE_WIDTH_SECONDS: f64 = 0.016;
const TIMELINE_MIPMAP_COARSE_RATIO: i64 = 16;
const TIMELINE_MIPMAP_MAX_BINS: i64 = 256;

#[derive(Debug, Parser)]
#[command(about = "Query-backed modern workspace for KUtrace v3 JSON")]
struct Args {
    /// KUtrace version-3 JSON produced by eventtospan3.
    trace: PathBuf,
    /// HTTP address for the local workspace.
    #[arg(long, default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
    /// Existing self-contained KUtrace HTML served at /legacy.
    #[arg(long)]
    legacy_html: Option<PathBuf>,
    /// Persistent SQLite database. By default an adjacent .sqlite file is used.
    #[arg(long)]
    database: Option<PathBuf>,
    /// Rebuild the database even when it is newer than the source JSON.
    #[arg(long)]
    rebuild: bool,
    /// Build/validate the SQLite index and exit without starting HTTP.
    #[arg(long)]
    import_only: bool,
    /// Auxiliary SQLite workers used while building indexes. Zero keeps peak
    /// import memory low; use a positive value to trade memory for index speed.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..=64))]
    index_workers: i64,
}

#[derive(Clone)]
struct AppState {
    database: PathBuf,
    legacy_html: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct QueryRequest {
    sql: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

const fn default_limit() -> usize {
    1_000
}

#[derive(Debug, Serialize)]
struct QueryResponse {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
    truncated: bool,
    elapsed_ms: f64,
    sql: String,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

fn database_path(args: &Args) -> PathBuf {
    args.database
        .clone()
        .unwrap_or_else(|| args.trace.with_extension("kutrace.sqlite"))
}

fn database_is_current(database: &Path, trace: &Path) -> bool {
    let Ok(database_time) = database.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    let Ok(trace_time) = trace.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    if database_time < trace_time {
        return false;
    }
    let Ok(connection) = open_read_only(database) else {
        return false;
    };
    connection
        .query_row(
            "SELECT value FROM metadata WHERE key='ui_schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .is_ok_and(|value| value == UI_SCHEMA_VERSION.to_string())
}

fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode=OFF;
         PRAGMA synchronous=OFF;
         PRAGMA locking_mode=EXCLUSIVE;
         CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE events(
           id INTEGER PRIMARY KEY,
           ts REAL NOT NULL,
           dur REAL NOT NULL,
           ts_end REAL NOT NULL,
           cpu INTEGER NOT NULL,
           pid INTEGER NOT NULL,
           rpc INTEGER NOT NULL,
           event INTEGER NOT NULL,
           arg0 INTEGER NOT NULL,
           retval INTEGER NOT NULL,
           ipc INTEGER NOT NULL,
           name TEXT NOT NULL,
           category TEXT NOT NULL
         );
         CREATE TABLE timeline_mipmap(
           bucket INTEGER NOT NULL,
           bucket_start REAL NOT NULL,
           bucket_end REAL NOT NULL,
           cpu INTEGER NOT NULL,
           event INTEGER NOT NULL,
           name TEXT NOT NULL,
           category TEXT NOT NULL,
           ipc INTEGER NOT NULL,
           weight REAL NOT NULL,
           count INTEGER NOT NULL
         );
         CREATE TABLE timeline_mipmap_coarse(
           bucket INTEGER NOT NULL,
           bucket_start REAL NOT NULL,
           bucket_end REAL NOT NULL,
           cpu INTEGER NOT NULL,
           event INTEGER NOT NULL,
           name TEXT NOT NULL,
           category TEXT NOT NULL,
           ipc INTEGER NOT NULL,
           weight REAL NOT NULL,
           count INTEGER NOT NULL
         );
         CREATE TABLE profile_samples(
           sample_id INTEGER PRIMARY KEY,
           event_id INTEGER NOT NULL REFERENCES events(id),
           ts REAL NOT NULL,
           cpu INTEGER NOT NULL,
           pid INTEGER NOT NULL,
           stack_depth INTEGER NOT NULL,
           has_callchain INTEGER NOT NULL
         );
         CREATE TABLE profile_frames(
           sample_id INTEGER NOT NULL REFERENCES profile_samples(sample_id),
           depth INTEGER NOT NULL,
           name TEXT NOT NULL,
           PRIMARY KEY(sample_id, depth)
         );",
    )?;
    Ok(())
}

fn finalize_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(&format!(
        "WITH RECURSIVE expanded(bucket,last_bucket,cpu,event,name,category,ipc,ts,ts_end) AS (
           SELECT CAST(ts/{width} AS INTEGER),
                  CAST((ts_end-0.000000000001)/{width} AS INTEGER),
                  cpu,event,name,category,ipc,ts,ts_end
             FROM events
            WHERE dur>0 AND dur<={max_duration}
           UNION ALL
           SELECT bucket+1,last_bucket,cpu,event,name,category,ipc,ts,ts_end
             FROM expanded WHERE bucket<last_bucket
         )
         INSERT INTO timeline_mipmap(bucket,bucket_start,bucket_end,cpu,event,name,category,ipc,weight,count)
         SELECT bucket,bucket*{width},(bucket+1)*{width},cpu,event,'',category,ipc,
                SUM(MIN(ts_end,(bucket+1)*{width})-MAX(ts,bucket*{width})),COUNT(*)
          FROM expanded
          GROUP BY bucket,cpu,event,category,ipc;
         INSERT INTO timeline_mipmap_coarse(bucket,bucket_start,bucket_end,cpu,event,name,category,ipc,weight,count)
         SELECT CAST(bucket/{coarse_ratio} AS INTEGER),
                CAST(bucket/{coarse_ratio} AS INTEGER)*{coarse_width},
                (CAST(bucket/{coarse_ratio} AS INTEGER)+1)*{coarse_width},
                cpu,event,'',category,ipc,SUM(weight),SUM(count)
           FROM timeline_mipmap
          GROUP BY CAST(bucket/{coarse_ratio} AS INTEGER),cpu,event,category,ipc;
         CREATE INDEX timeline_mipmap_time ON timeline_mipmap(bucket_start,bucket_end,cpu);
         CREATE INDEX timeline_mipmap_coarse_time ON timeline_mipmap_coarse(bucket_start,bucket_end,cpu);
         CREATE INDEX events_timeline ON events(ts, ts_end, cpu, event, category, ipc, name);
         CREATE INDEX events_cpu_ts ON events(cpu, ts);
         CREATE INDEX events_pid_ts ON events(pid, ts);
         CREATE INDEX events_category_ts ON events(category, ts);
         CREATE INDEX events_event_ts ON events(event, ts);
         CREATE INDEX events_rpc_ts ON events(rpc, ts) WHERE rpc != 0;
         CREATE INDEX events_retval_ts ON events(retval, ts)
           WHERE retval > 0 AND event BETWEEN 522 AND 525;
         CREATE INDEX events_name_ts ON events(name, ts);
         INSERT INTO profile_samples(sample_id,event_id,ts,cpu,pid,stack_depth,has_callchain)
         SELECT id,id,ts,cpu,pid,0,INSTR(name,';')>0
           FROM events WHERE category='sample' AND name!='';
         WITH RECURSIVE frames(sample_id,depth,frame,rest) AS (
           SELECT id,-1,'',name||';' FROM events
            WHERE category='sample' AND name!=''
           UNION ALL
           SELECT sample_id,depth+1,
                  SUBSTR(rest,1,INSTR(rest,';')-1),
                  SUBSTR(rest,INSTR(rest,';')+1)
             FROM frames WHERE rest!='' AND depth<127
         )
         INSERT INTO profile_frames(sample_id,depth,name)
         SELECT sample_id,depth,frame FROM frames WHERE frame!='';
         UPDATE profile_samples
            SET stack_depth=(SELECT COUNT(*) FROM profile_frames
                              WHERE profile_frames.sample_id=profile_samples.sample_id);
         CREATE INDEX profile_samples_ts ON profile_samples(ts,cpu,pid);
         CREATE INDEX profile_frames_name ON profile_frames(name,sample_id,depth);
         CREATE VIEW agent_spans AS
           SELECT id, ts, dur, ts_end, cpu, pid,
                  arg0 AS span_id, retval AS parent_span_id, name
           FROM events WHERE event = 645;
         CREATE VIEW rpc_activity AS
           SELECT id, ts, dur, ts_end, cpu, pid,
                  CASE WHEN rpc != 0 THEN rpc ELSE arg0 END AS rpc_id,
                  event, arg0, retval, ipc, name, category
           FROM events WHERE rpc != 0 OR event BETWEEN 513 AND 517;
         CREATE VIEW resource_activity AS
           SELECT id, ts, dur, ts_end, cpu, pid, rpc, event, arg0, retval,
                  ipc, name, category
           FROM events
           WHERE event BETWEEN 528 AND 530 OR event BETWEEN 537 AND 539;
         CREATE VIEW agent_annotations AS
           SELECT id, ts, dur, ts_end, cpu, pid, retval AS span_id,
                  arg0 AS value, event, name,
                  CASE event
                    WHEN 522 THEN 'query'
                    WHEN 523 THEN 'observation'
                    WHEN 524 THEN 'decision'
                    WHEN 525 THEN 'result'
                  END AS annotation_kind
           FROM events
           WHERE retval > 0 AND (
             (event = 522 AND (name = 'agent.query' OR name LIKE 'agent.query.%')) OR
             (event = 523 AND (name = 'agent.observation' OR name LIKE 'agent.observation.%')) OR
             (event = 524 AND (name = 'agent.decision' OR name LIKE 'agent.decision.%')) OR
             (event = 525 AND (name = 'agent.result' OR name LIKE 'agent.result.%'))
           );
         CREATE VIEW event_summary AS
           SELECT event, name, COUNT(*) AS count,
                  SUM(dur) AS total_duration,
                  AVG(dur) AS average_duration
           FROM events GROUP BY event, name;
         CREATE VIEW profile_callchains AS
           SELECT sample.sample_id,sample.ts,sample.cpu,sample.pid,
                  sample.stack_depth,sample.has_callchain,frame.depth,frame.name
             FROM profile_samples sample JOIN profile_frames frame USING(sample_id);
         INSERT OR REPLACE INTO metadata(key,value) VALUES
           ('timeline_mipmap_width','{width}'),
           ('timeline_mipmap_coarse_width','{coarse_width}'),
           ('timeline_mipmap_max_bins','{max_bins}');
         PRAGMA optimize;",
        width = TIMELINE_MIPMAP_WIDTH_SECONDS,
        coarse_width = TIMELINE_MIPMAP_COARSE_WIDTH_SECONDS,
        coarse_ratio = TIMELINE_MIPMAP_COARSE_RATIO,
        max_bins = TIMELINE_MIPMAP_MAX_BINS,
        max_duration = TIMELINE_MIPMAP_WIDTH_SECONDS * TIMELINE_MIPMAP_MAX_BINS as f64,
    ))?;
    Ok(())
}

fn is_agent_annotation(event: i64, retval: i64, name: &str) -> bool {
    let prefix = match event {
        0x20a => "agent.query",
        0x20b => "agent.observation",
        0x20c => "agent.decision",
        0x20d => "agent.result",
        _ => return false,
    };
    retval > 0
        && name
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
}

fn category(event: i64, retval: i64, name: &str) -> &'static str {
    if is_agent_annotation(event, retval, name) {
        return "annotation";
    }
    match event {
        0x285 => "agent",
        0x800..=0xfff => "syscall",
        0x400..=0x7ff => "kernel",
        0x200 => "scheduler",
        0x201..=0x205 => "rpc",
        0x206 => "wakeup",
        0x20a..=0x20d => "mark",
        0x210..=0x212 => "lock",
        0x219..=0x21b => "resource",
        0x280..=0x281 => "sample",
        0x282..=0x283 => "lock",
        value if value > 0xffff => "user",
        _ => "special",
    }
}

#[derive(Debug)]
struct ImportSummary {
    version: Option<i64>,
    event_count: u64,
    events_present: bool,
    load_ms: f64,
    index_ms: f64,
    index_workers: i64,
}

struct EventsSeed<'transaction, 'connection> {
    transaction: &'transaction Transaction<'connection>,
}

impl<'de> DeserializeSeed<'de> for EventsSeed<'_, '_> {
    type Value = u64;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(EventsVisitor {
            transaction: self.transaction,
        })
    }
}

struct EventsVisitor<'transaction, 'connection> {
    transaction: &'transaction Transaction<'connection>,
}

fn insert_event_batch(transaction: &Transaction<'_>, events: &mut Vec<RawEvent>) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let mut sql = String::from(
        "INSERT INTO events(ts,dur,ts_end,cpu,pid,rpc,event,arg0,retval,ipc,name,category) VALUES ",
    );
    for index in 0..events.len() {
        if index != 0 {
            sql.push(',');
        }
        sql.push_str("(?,?,?,?,?,?,?,?,?,?,?,?)");
    }
    let mut values = Vec::with_capacity(events.len() * 12);
    for (ts, dur, cpu, pid, rpc, event, arg0, retval, ipc, name) in events.drain(..) {
        let category = category(event, retval, &name).to_owned();
        values.extend([
            rusqlite::types::Value::Real(ts),
            rusqlite::types::Value::Real(dur),
            rusqlite::types::Value::Real(ts + dur),
            rusqlite::types::Value::Integer(cpu),
            rusqlite::types::Value::Integer(pid),
            rusqlite::types::Value::Integer(rpc),
            rusqlite::types::Value::Integer(event),
            rusqlite::types::Value::Integer(arg0),
            rusqlite::types::Value::Integer(retval),
            rusqlite::types::Value::Integer(ipc),
            rusqlite::types::Value::Text(name),
            rusqlite::types::Value::Text(category),
        ]);
    }
    transaction
        .prepare_cached(&sql)?
        .execute(params_from_iter(values))?;
    Ok(())
}

impl<'de> Visitor<'de> for EventsVisitor<'_, '_> {
    type Value = u64;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the KUtrace events array")
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut count = 0;
        let mut events = Vec::with_capacity(EVENT_INSERT_BATCH);
        while let Some((ts, dur, cpu, pid, rpc, event, arg0, retval, ipc, name)) =
            sequence.next_element::<RawEvent>()?
        {
            if ts == 999.0 && dur == 0.0 && event == 0 && name.is_empty() {
                continue;
            }
            events.push((ts, dur, cpu, pid, rpc, event, arg0, retval, ipc, name));
            if events.len() == EVENT_INSERT_BATCH {
                insert_event_batch(self.transaction, &mut events).map_err(de::Error::custom)?;
            }
            count += 1;
        }
        insert_event_batch(self.transaction, &mut events).map_err(de::Error::custom)?;
        Ok(count)
    }
}

struct TraceSeed<'transaction, 'connection> {
    transaction: &'transaction Transaction<'connection>,
}

impl<'de> DeserializeSeed<'de> for TraceSeed<'_, '_> {
    type Value = ImportSummary;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(TraceVisitor {
            transaction: self.transaction,
        })
    }
}

struct TraceVisitor<'transaction, 'connection> {
    transaction: &'transaction Transaction<'connection>,
}

impl<'de> Visitor<'de> for TraceVisitor<'_, '_> {
    type Value = ImportSummary;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a KUtrace version-3 JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut version = None;
        let mut event_count = 0;
        let mut events_present = false;
        while let Some(key) = map.next_key::<String>()? {
            if key == "events" {
                if events_present {
                    return Err(de::Error::duplicate_field("events"));
                }
                events_present = true;
                event_count = map.next_value_seed(EventsSeed {
                    transaction: self.transaction,
                })?;
                continue;
            }
            let value = map.next_value::<Value>()?;
            if key == "version" {
                version = value.as_i64();
            }
            let text = match value {
                Value::String(value) => value,
                value => value.to_string(),
            };
            self.transaction
                .execute(
                    "INSERT OR REPLACE INTO metadata(key,value) VALUES(?1,?2)",
                    params![key, text],
                )
                .map_err(de::Error::custom)?;
        }
        Ok(ImportSummary {
            version,
            event_count,
            events_present,
            load_ms: 0.0,
            index_ms: 0.0,
            index_workers: 0,
        })
    }
}

fn staging_path(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_owned();
    path.push(".tmp");
    PathBuf::from(path)
}

fn configure_index_workers(connection: &Connection, workers: i64) -> Result<i64> {
    connection.pragma_update(None, "threads", workers)?;
    Ok(connection.pragma_query_value(None, "threads", |row| row.get(0))?)
}

fn import_to_staging(
    trace_path: &Path,
    staging: &Path,
    index_workers: i64,
) -> Result<ImportSummary> {
    let source = std::fs::File::open(trace_path)
        .with_context(|| format!("open trace {}", trace_path.display()))?;
    let mut connection = Connection::open(staging)?;
    create_schema(&connection)?;
    let transaction = connection.transaction()?;
    let load_started = std::time::Instant::now();
    let mut deserializer =
        serde_json::Deserializer::from_reader(BufReader::with_capacity(256 * 1024, source));
    let mut summary = TraceSeed {
        transaction: &transaction,
    }
    .deserialize(&mut deserializer)
    .with_context(|| format!("parse trace {}", trace_path.display()))?;
    if summary.version != Some(3) {
        bail!(
            "expected KUtrace JSON version 3, got {}",
            summary
                .version
                .map_or_else(|| "missing".to_owned(), |value| value.to_string())
        );
    }
    if !summary.events_present {
        bail!("KUtrace JSON is missing the events array");
    }
    transaction.execute(
        "INSERT OR REPLACE INTO metadata(key,value) VALUES('event_count',?1),('ui_schema_version',?2)",
        params![
            summary.event_count.to_string(),
            UI_SCHEMA_VERSION.to_string()
        ],
    )?;
    transaction.commit()?;
    summary.load_ms = load_started.elapsed().as_secs_f64() * 1_000.0;
    summary.index_workers = configure_index_workers(&connection, index_workers)?;
    let index_started = std::time::Instant::now();
    finalize_schema(&connection)?;
    summary.index_ms = index_started.elapsed().as_secs_f64() * 1_000.0;
    drop(connection);
    Ok(summary)
}

fn import_trace(trace_path: &Path, database: &Path, index_workers: i64) -> Result<ImportSummary> {
    let staging = staging_path(database);
    if staging.exists() {
        std::fs::remove_file(&staging)
            .with_context(|| format!("remove stale staging database {}", staging.display()))?;
    }
    let result = import_to_staging(trace_path, &staging, index_workers);
    let summary = match result {
        Ok(summary) => summary,
        Err(error) => {
            if staging.exists() {
                let _ = std::fs::remove_file(&staging);
            }
            return Err(error);
        }
    };
    std::fs::rename(&staging, database).with_context(|| {
        format!(
            "publish staging database {} as {}",
            staging.display(),
            database.display()
        )
    })?;
    Ok(summary)
}

fn open_read_only(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
    Ok(connection)
}

fn sqlite_value(value: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::from(value),
        ValueRef::Real(value) => Value::from(value),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => Value::String(format!("<{} byte blob>", value.len())),
    }
}

fn run_query(database: &Path, request: QueryRequest) -> Result<QueryResponse> {
    let limit = request.limit.clamp(1, 50_000);
    let connection = open_read_only(database)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    connection.progress_handler(10_000, Some(move || std::time::Instant::now() >= deadline))?;
    let started = std::time::Instant::now();
    let mut statement = connection.prepare(&request.sql)?;
    if !statement.readonly() {
        bail!("only read-only SQL is permitted");
    }
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let column_count = columns.len();
    let mut cursor = statement.query([])?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.next()? {
        if rows.len() == limit {
            return Ok(QueryResponse {
                columns,
                rows,
                truncated: true,
                elapsed_ms: started.elapsed().as_secs_f64() * 1_000.0,
                sql: request.sql,
            });
        }
        let mut values = Vec::with_capacity(column_count);
        for column in 0..column_count {
            values.push(sqlite_value(row.get_ref(column)?));
        }
        rows.push(values);
    }
    Ok(QueryResponse {
        columns,
        rows,
        truncated: false,
        elapsed_ms: started.elapsed().as_secs_f64() * 1_000.0,
        sql: request.sql,
    })
}

async fn query(
    State(state): State<AppState>,
    Json(request): Json<QueryRequest>,
) -> impl IntoResponse {
    let database = state.database.clone();
    match tokio::task::spawn_blocking(move || run_query(&database, request)).await {
        Ok(Ok(response)) => (StatusCode::OK, Json(response)).into_response(),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: format!("{error:#}"),
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn schema(State(state): State<AppState>) -> impl IntoResponse {
    let request = QueryRequest {
        sql: "SELECT type, name, sql FROM sqlite_schema WHERE type IN ('table','view') ORDER BY type,name".to_owned(),
        limit: 1_000,
    };
    query(State(state), Json(request)).await
}

async fn index() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/wasm/index.html"),
    )
}

async fn wasm_javascript() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/wasm/kutrace-ui-web.js"),
    )
}

async fn wasm_binary() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/wasm"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_bytes!("../assets/wasm/kutrace-ui-web_bg.wasm").as_slice(),
    )
}

async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/style.css"),
    )
}

async fn legacy_keyboard() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/legacy-keyboard.js"),
    )
}

async fn legacy(State(state): State<AppState>) -> Response {
    let Some(path) = state.legacy_html else {
        return (StatusCode::NOT_FOUND, "No --legacy-html was supplied").into_response();
    };
    match std::fs::read(path) {
        Ok(mut bytes) => {
            bytes.extend_from_slice(b"\n<script src=\"/legacy-keyboard.js\"></script>\n");
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .body(Body::from(bytes))
                .unwrap()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let database = database_path(&args);
    let started = std::time::Instant::now();
    let mut imported = None;
    if args.rebuild || !database_is_current(&database, &args.trace) {
        imported = Some(import_trace(&args.trace, &database, args.index_workers)?);
    }
    if args.import_only {
        let (count, load_ms, index_ms, index_workers) = if let Some(summary) = imported {
            (
                summary.event_count,
                Some(summary.load_ms),
                Some(summary.index_ms),
                Some(summary.index_workers),
            )
        } else {
            let connection = open_read_only(&database)?;
            (
                connection.query_row("SELECT COUNT(*) FROM events", [], |row| {
                    row.get::<_, i64>(0)
                })? as u64,
                None,
                None,
                None,
            )
        };
        println!(
            "{{\"database\":{},\"events\":{},\"elapsed_ms\":{:.3},\"load_ms\":{},\"index_ms\":{},\"index_workers\":{}}}",
            serde_json::to_string(&database.display().to_string())?,
            count,
            started.elapsed().as_secs_f64() * 1_000.0,
            serde_json::to_string(&load_ms)?,
            serde_json::to_string(&index_ms)?,
            serde_json::to_string(&index_workers)?,
        );
        return Ok(());
    }
    let state = AppState {
        database,
        legacy_html: args.legacy_html,
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/wasm/kutrace-ui-web.js", get(wasm_javascript))
        .route("/wasm/kutrace-ui-web_bg.wasm", get(wasm_binary))
        .route("/style.css", get(stylesheet))
        .route("/legacy", get(legacy))
        .route("/legacy-keyboard.js", get(legacy_keyboard))
        .route("/api/query", post(query))
        .route("/api/schema", get(schema))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    eprintln!("KUtrace workspace: http://{}", args.listen);
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hello_world_demo.json")
    }

    #[test]
    fn imports_real_v3_trace_and_enforces_read_only_bounded_queries() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("trace.sqlite");
        let summary = import_trace(&fixture(), &database, 0).unwrap();
        assert_eq!(summary.index_workers, 0);
        assert!(summary.load_ms > 0.0);
        assert!(summary.index_ms > 0.0);

        let count = run_query(
            &database,
            QueryRequest {
                sql: "SELECT COUNT(*) FROM events".to_owned(),
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(count.rows, vec![vec![Value::from(38_696)]]);

        let indexes = run_query(
            &database,
            QueryRequest {
                sql: "SELECT name,sql FROM sqlite_schema WHERE type='index' AND name LIKE 'events_%' ORDER BY name".to_owned(),
                limit: 20,
            },
        )
        .unwrap();
        assert_eq!(indexes.rows.len(), 8);
        assert!(indexes.rows.iter().any(|row| {
            row[0] == "events_category_ts"
                && row[1]
                    .as_str()
                    .is_some_and(|sql| sql.contains("category, ts"))
        }));
        assert!(indexes.rows.iter().any(|row| {
            row[0] == "events_rpc_ts"
                && row[1]
                    .as_str()
                    .is_some_and(|sql| sql.contains("WHERE rpc != 0"))
        }));
        assert!(indexes.rows.iter().any(|row| {
            row[0] == "events_retval_ts"
                && row[1]
                    .as_str()
                    .is_some_and(|sql| sql.contains("event BETWEEN 522 AND 525"))
        }));

        let mipmap = run_query(
            &database,
            QueryRequest {
                sql: "SELECT
                        (SELECT COUNT(*) FROM timeline_mipmap) > 0,
                        ABS(
                          (SELECT SUM(dur) FROM events WHERE dur>0 AND dur<=0.256) -
                          (SELECT SUM(weight) FROM timeline_mipmap)
                        ) < 0.000001,
                        (SELECT COUNT(DISTINCT name) FROM timeline_mipmap) = 1,
                        (SELECT MAX(name) FROM timeline_mipmap) = '',
                        ABS(
                          (SELECT SUM(weight) FROM timeline_mipmap) -
                          (SELECT SUM(weight) FROM timeline_mipmap_coarse)
                        ) < 0.000001,
                        (SELECT COUNT(DISTINCT name) FROM timeline_mipmap_coarse) = 1,
                        (SELECT MAX(name) FROM timeline_mipmap_coarse) = '',
                        (SELECT value FROM metadata WHERE key='timeline_mipmap_width')"
                    .to_owned(),
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(
            mipmap.rows,
            vec![vec![
                Value::from(1),
                Value::from(1),
                Value::from(1),
                Value::from(1),
                Value::from(1),
                Value::from(1),
                Value::from(1),
                Value::from("0.001"),
            ]]
        );

        let relationship_views = run_query(
            &database,
            QueryRequest {
                sql: "SELECT name FROM sqlite_schema WHERE type='view' AND name IN ('agent_annotations','rpc_activity','resource_activity') ORDER BY name".to_owned(),
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(
            relationship_views.rows,
            vec![
                vec![Value::from("agent_annotations")],
                vec![Value::from("resource_activity")],
                vec![Value::from("rpc_activity")],
            ]
        );
        assert_eq!(category(0x201, 0, "rpc"), "rpc");
        assert_eq!(category(0x20a, 1, "agent.query.sql"), "annotation");
        assert_eq!(category(0x20a, 1, "ordinary.mark"), "mark");
        assert_eq!(category(0x20a, 0, "agent.query.sql"), "mark");
        assert_eq!(category(0x210, 0, "lock"), "lock");
        assert_eq!(category(0x280, 0, "user-pc"), "sample");
        assert_eq!(category(0x281, 0, "kernel-pc"), "sample");
        assert_eq!(category(0x282, 0, "lock-held"), "lock");
        assert_eq!(category(0x283, 0, "lock-try"), "lock");
        assert_eq!(category(0x219, 0, "resource"), "resource");

        let bounded = run_query(
            &database,
            QueryRequest {
                sql: "SELECT id FROM events ORDER BY id".to_owned(),
                limit: 3,
            },
        )
        .unwrap();
        assert_eq!(bounded.rows.len(), 3);
        assert!(bounded.truncated);

        let write = run_query(
            &database,
            QueryRequest {
                sql: "DELETE FROM events".to_owned(),
                limit: 10,
            },
        );
        assert!(write.is_err());

        let invalid = directory.path().join("invalid.json");
        std::fs::write(
            &invalid,
            r#"{"version":2,"events":[[1.0,0.1,0,1,0,645,1,0,0,"bad"]]}"#,
        )
        .unwrap();
        assert!(import_trace(&invalid, &database, 0).is_err());
        assert!(!staging_path(&database).exists());
        let preserved = run_query(
            &database,
            QueryRequest {
                sql: "SELECT COUNT(*) FROM events".to_owned(),
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(preserved.rows, vec![vec![Value::from(38_696)]]);
    }

    #[test]
    fn imports_symbolized_sample_callchains_into_normalized_profile_tables() {
        let directory = tempfile::tempdir().unwrap();
        let trace = directory.path().join("stacks.json");
        let database = directory.path().join("stacks.sqlite");
        std::fs::write(
            &trace,
            r#"{"version":3,"title":"stacks","events":[[0.1,0.000001,0,42,0,640,0,0,0,"main;work;leaf"]]}"#,
        )
        .unwrap();
        import_trace(&trace, &database, 0).unwrap();
        let connection = open_read_only(&database).unwrap();
        let sample = connection
            .query_row(
                "SELECT stack_depth,has_callchain FROM profile_samples",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        assert_eq!(sample, (3, 1));
        let frames = connection
            .prepare("SELECT depth,name FROM profile_frames ORDER BY depth")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            frames,
            vec![
                (0, "main".to_owned()),
                (1, "work".to_owned()),
                (2, "leaf".to_owned())
            ]
        );
    }
}
