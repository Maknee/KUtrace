use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub title: String,
    pub count: u64,
    pub flags: u64,
    pub full: Range,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

impl Range {
    pub fn span(self) -> f64 {
        (self.end - self.start).max(f64::EPSILON)
    }

    pub fn bounded(self, full: Range) -> Self {
        if !self.start.is_finite() || !self.end.is_finite() || self.end <= self.start {
            return full;
        }
        let width = self.span();
        if width >= full.span() {
            return full;
        }
        let mut start = self.start;
        let mut end = self.end;
        if start < full.start {
            end += full.start - start;
            start = full.start;
        }
        if end > full.end {
            start -= end - full.end;
            end = full.end;
        }
        Self {
            start: start.max(full.start),
            end: end.min(full.end),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub field: String,
    pub op: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackMode {
    #[default]
    CpuPid,
    Cpu,
    Pid,
}

impl TrackMode {
    pub fn value(self) -> &'static str {
        match self {
            Self::CpuPid => "cpu_pid",
            Self::Cpu => "cpu",
            Self::Pid => "pid",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackGroups {
    pub cpu: bool,
    pub pid: bool,
    pub rpc: bool,
    pub resource: bool,
}

impl TrackGroups {
    pub const fn for_mode(mode: TrackMode) -> Self {
        match mode {
            TrackMode::CpuPid => Self {
                cpu: true,
                pid: true,
                rpc: true,
                resource: true,
            },
            TrackMode::Cpu => Self {
                cpu: true,
                pid: false,
                rpc: false,
                resource: false,
            },
            TrackMode::Pid => Self {
                cpu: false,
                pid: true,
                rpc: false,
                resource: false,
            },
        }
    }

    pub fn enabled(self, name: &str) -> bool {
        match name {
            "cpu" => self.cpu,
            "pid" => self.pid,
            "rpc" => self.rpc,
            "resource" => self.resource,
            _ => false,
        }
    }

    pub fn toggle(&mut self, name: &str) {
        match name {
            "cpu" => self.cpu = !self.cpu,
            "pid" => self.pid = !self.pid,
            "rpc" => self.rpc = !self.rpc,
            "resource" => self.resource = !self.resource,
            _ => {}
        }
    }

    pub const fn count(self) -> usize {
        self.cpu as usize + self.pid as usize + self.rpc as usize + self.resource as usize
    }

    pub fn names(self) -> String {
        ["cpu", "pid", "rpc", "resource"]
            .into_iter()
            .filter(|name| self.enabled(name))
            .collect::<Vec<_>>()
            .join(",")
    }
}

impl Default for TrackGroups {
    fn default() -> Self {
        Self::for_mode(TrackMode::CpuPid)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceEvent {
    pub id: i64,
    pub start: f64,
    pub duration: f64,
    pub end: f64,
    pub cpu: i64,
    pub pid: i64,
    pub rpc: i64,
    pub event: i64,
    pub name: String,
    pub category: String,
    pub arg0: i64,
    pub retval: i64,
    pub ipc: i64,
    /// Synthetic density rows belong to one explicit rendered track. Exact
    /// events leave this unset and are projected into both CPU and PID rows.
    pub render_track: Option<String>,
}

fn i64_at(row: &[Value], index: usize) -> i64 {
    row.get(index).and_then(Value::as_i64).unwrap_or_default()
}

fn f64_at(row: &[Value], index: usize) -> f64 {
    row.get(index).and_then(Value::as_f64).unwrap_or_default()
}

fn string_at(row: &[Value], index: usize) -> String {
    row.get(index)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl TraceEvent {
    pub fn from_row(row: &[Value]) -> Self {
        Self {
            id: i64_at(row, 0),
            start: f64_at(row, 1),
            duration: f64_at(row, 2),
            end: f64_at(row, 3),
            cpu: i64_at(row, 4),
            pid: i64_at(row, 5),
            rpc: i64_at(row, 6),
            event: i64_at(row, 7),
            name: string_at(row, 8),
            category: string_at(row, 9),
            arg0: i64_at(row, 10),
            retval: i64_at(row, 11),
            ipc: i64_at(row, 12),
            render_track: row.get(13).and_then(Value::as_str).map(str::to_owned),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlays {
    pub marks: bool,
    pub arcs: bool,
    pub locks: bool,
    pub frequency: bool,
    pub ipc: bool,
    pub samples: bool,
    pub colorblind: bool,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            marks: true,
            arcs: true,
            locks: true,
            frequency: true,
            ipc: true,
            samples: true,
            colorblind: false,
        }
    }
}

pub fn sql_literal(value: &str) -> String {
    if value.parse::<f64>().is_ok() {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

pub fn filter_sql(filters: &[Filter]) -> String {
    filters
        .iter()
        .map(|filter| {
            if filter.op == "contains" {
                format!(
                    "{} LIKE '%' || {} || '%'",
                    filter.field,
                    sql_literal(&filter.value)
                )
            } else {
                format!(
                    "{} {} {}",
                    filter.field,
                    filter.op,
                    sql_literal(&filter.value)
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn where_sql(filters: &[Filter], extra: &str) -> String {
    let mut parts = Vec::new();
    let filters = filter_sql(filters);
    if !filters.is_empty() {
        parts.push(filters);
    }
    if !extra.is_empty() {
        parts.push(extra.to_owned());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", parts.join(" AND "))
    }
}
