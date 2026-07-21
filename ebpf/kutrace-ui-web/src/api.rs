use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct QueryResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub elapsed_ms: f64,
    pub sql: String,
}

#[derive(Serialize)]
struct QueryRequest<'a> {
    sql: &'a str,
    limit: usize,
}

pub async fn query(sql: &str, limit: usize) -> Result<QueryResponse, String> {
    let response = Request::post("/api/query")
        .header("content-type", "application/json")
        .json(&QueryRequest { sql, limit })
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    if !(200..300).contains(&status) {
        let message = response
            .json::<Value>()
            .await
            .ok()
            .and_then(|body| body.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("query failed with HTTP {status}"));
        return Err(message);
    }
    response
        .json::<QueryResponse>()
        .await
        .map_err(|error| error.to_string())
}
