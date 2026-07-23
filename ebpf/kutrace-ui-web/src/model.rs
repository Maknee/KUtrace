use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, Visitor},
};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackGroupMode {
    Hidden,
    Highlighted,
    Full,
}

impl TrackGroupMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Highlighted => "highlighted",
            Self::Full => "full",
        }
    }
}

impl<'de> Deserialize<'de> for TrackGroupMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ModeVisitor;

        impl Visitor<'_> for ModeVisitor {
            type Value = TrackGroupMode;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a group mode or a legacy boolean")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(if value {
                    TrackGroupMode::Full
                } else {
                    TrackGroupMode::Hidden
                })
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "hidden" => Ok(TrackGroupMode::Hidden),
                    "highlighted" => Ok(TrackGroupMode::Highlighted),
                    "full" => Ok(TrackGroupMode::Full),
                    _ => Err(E::unknown_variant(
                        value,
                        &["hidden", "highlighted", "full"],
                    )),
                }
            }
        }

        deserializer.deserialize_any(ModeVisitor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackGroups {
    pub cpu: TrackGroupMode,
    pub pid: TrackGroupMode,
    pub rpc: TrackGroupMode,
    pub resource: TrackGroupMode,
}

impl TrackGroups {
    pub const fn for_mode(mode: TrackMode) -> Self {
        match mode {
            TrackMode::CpuPid => Self {
                cpu: TrackGroupMode::Full,
                pid: TrackGroupMode::Full,
                rpc: TrackGroupMode::Full,
                resource: TrackGroupMode::Full,
            },
            TrackMode::Cpu => Self {
                cpu: TrackGroupMode::Full,
                pid: TrackGroupMode::Hidden,
                rpc: TrackGroupMode::Hidden,
                resource: TrackGroupMode::Hidden,
            },
            TrackMode::Pid => Self {
                cpu: TrackGroupMode::Hidden,
                pid: TrackGroupMode::Full,
                rpc: TrackGroupMode::Hidden,
                resource: TrackGroupMode::Hidden,
            },
        }
    }

    pub fn mode(self, name: &str) -> TrackGroupMode {
        match name {
            "cpu" => self.cpu,
            "pid" => self.pid,
            "rpc" => self.rpc,
            "resource" => self.resource,
            _ => TrackGroupMode::Hidden,
        }
    }

    pub fn enabled(self, name: &str) -> bool {
        self.mode(name) != TrackGroupMode::Hidden
    }

    pub fn cycle(&mut self, name: &str, has_highlight: bool) {
        let mode = self.mode(name);
        let next = match (mode, has_highlight) {
            (TrackGroupMode::Full, true) => TrackGroupMode::Highlighted,
            (TrackGroupMode::Highlighted, true) => TrackGroupMode::Hidden,
            (TrackGroupMode::Hidden, _) => TrackGroupMode::Full,
            (TrackGroupMode::Full | TrackGroupMode::Highlighted, false) => TrackGroupMode::Hidden,
        };
        match name {
            "cpu" => self.cpu = next,
            "pid" => self.pid = next,
            "rpc" => self.rpc = next,
            "resource" => self.resource = next,
            _ => {}
        }
    }

    pub fn count(self) -> usize {
        [self.cpu, self.pid, self.rpc, self.resource]
            .into_iter()
            .filter(|mode| *mode != TrackGroupMode::Hidden)
            .count()
    }

    pub fn names(self) -> String {
        ["cpu", "pid", "rpc", "resource"]
            .into_iter()
            .filter(|name| self.enabled(name))
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn states(self) -> String {
        ["cpu", "pid", "rpc", "resource"]
            .into_iter()
            .map(|name| format!("{name}:{}", self.mode(name).as_str()))
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

fn deserialize_mode<'de, D, const MAX: u8>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    struct ModeVisitor<const MAX: u8>;

    impl<const MAX: u8> Visitor<'_> for ModeVisitor<MAX> {
        type Value = u8;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "a display mode from 0 through {MAX}, or a legacy boolean"
            )
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(if value { MAX } else { 0 })
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u8::try_from(value)
                .ok()
                .filter(|value| *value <= MAX)
                .ok_or_else(|| E::invalid_value(de::Unexpected::Unsigned(value), &self))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u64::try_from(value)
                .map_err(|_| E::invalid_value(de::Unexpected::Signed(value), &self))
                .and_then(|value| self.visit_u64(value))
        }
    }

    deserializer.deserialize_any(ModeVisitor::<MAX>)
}

fn deserialize_mode_2<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_mode::<D, 2>(deserializer)
}

fn deserialize_mode_3<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_mode::<D, 3>(deserializer)
}

const fn default_marks() -> u8 {
    3
}

const fn default_two() -> u8 {
    2
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlays {
    #[serde(default = "default_marks", deserialize_with = "deserialize_mode_3")]
    pub marks: u8,
    #[serde(default = "default_two", deserialize_with = "deserialize_mode_2")]
    pub arcs: u8,
    #[serde(default = "default_two", deserialize_with = "deserialize_mode_2")]
    pub locks: u8,
    #[serde(default = "default_two", deserialize_with = "deserialize_mode_2")]
    pub frequency: u8,
    #[serde(default, deserialize_with = "deserialize_mode_3")]
    pub ipc: u8,
    #[serde(default, deserialize_with = "deserialize_mode_2")]
    pub samples: u8,
    #[serde(default, deserialize_with = "deserialize_mode_2")]
    pub annotations: u8,
    pub colorblind: bool,
}

impl Overlays {
    pub fn level(self, name: &str) -> u8 {
        match name {
            "marks" => self.marks,
            "arcs" => self.arcs,
            "locks" => self.locks,
            "frequency" => self.frequency,
            "ipc" => self.ipc,
            "samples" => self.samples,
            "annotate_user" => u8::from(self.annotations == 1),
            "annotate_all" => u8::from(self.annotations == 2),
            "colorblind" => u8::from(self.colorblind),
            _ => 0,
        }
    }

    pub fn enabled(self, name: &str) -> bool {
        self.level(name) > 0
    }

    pub fn cycle(&mut self, name: &str, shifted: bool) {
        if name == "colorblind" {
            self.colorblind = !self.colorblind;
            return;
        }
        if name == "annotate_user" {
            self.annotations = if self.annotations == 1 { 0 } else { 1 };
            return;
        }
        if name == "annotate_all" {
            self.annotations = if self.annotations == 2 { 0 } else { 2 };
            return;
        }
        let (value, maximum, plain_toggle) = match name {
            "marks" => (&mut self.marks, 3, false),
            "arcs" => (&mut self.arcs, 2, false),
            "locks" => (&mut self.locks, 2, false),
            "frequency" => (&mut self.frequency, 2, false),
            "ipc" => (&mut self.ipc, 3, false),
            "samples" => (&mut self.samples, 2, true),
            _ => return,
        };
        if plain_toggle && !shifted {
            *value = if *value == 0 { maximum } else { 0 };
        } else {
            *value = if *value == 0 { maximum } else { *value - 1 };
        }
    }
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            marks: 3,
            arcs: 2,
            locks: 2,
            frequency: 2,
            ipc: 0,
            samples: 0,
            annotations: 0,
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

#[cfg(test)]
mod tests {
    use super::Overlays;

    #[test]
    fn display_modes_follow_original_cycles() {
        let mut overlays = Overlays::default();
        assert_eq!(
            (
                overlays.marks,
                overlays.arcs,
                overlays.locks,
                overlays.frequency,
                overlays.ipc,
                overlays.samples,
                overlays.annotations
            ),
            (3, 2, 2, 2, 0, 0, 0)
        );

        for expected in [2, 1, 0, 3] {
            overlays.cycle("marks", false);
            assert_eq!(overlays.marks, expected);
        }
        for expected in [3, 2, 1, 0] {
            overlays.cycle("ipc", false);
            assert_eq!(overlays.ipc, expected);
        }
        overlays.cycle("samples", false);
        assert_eq!(overlays.samples, 2);
        overlays.cycle("samples", false);
        assert_eq!(overlays.samples, 0);
        for expected in [2, 1, 0] {
            overlays.cycle("samples", true);
            assert_eq!(overlays.samples, expected);
        }

        overlays.cycle("annotate_user", false);
        assert_eq!(overlays.annotations, 1);
        overlays.cycle("annotate_all", false);
        assert_eq!(overlays.annotations, 2);
        overlays.cycle("annotate_all", false);
        assert_eq!(overlays.annotations, 0);
    }

    #[test]
    fn legacy_boolean_display_settings_migrate_to_maximum_modes() {
        let overlays: Overlays = serde_json::from_str(
            r#"{
                "marks": true,
                "arcs": false,
                "locks": true,
                "frequency": false,
                "ipc": true,
                "samples": true,
                "colorblind": true
            }"#,
        )
        .unwrap();
        assert_eq!(
            (
                overlays.marks,
                overlays.arcs,
                overlays.locks,
                overlays.frequency,
                overlays.ipc,
                overlays.samples,
                overlays.annotations,
                overlays.colorblind
            ),
            (3, 0, 2, 0, 3, 2, 0, true)
        );
    }
}
