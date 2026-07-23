use std::{
    collections::{BTreeMap, HashMap, HashSet},
    rc::Rc,
};

use gloo_events::EventListener;
use gloo_timers::callback::{Interval, Timeout};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Blob, BlobPropertyBag, HtmlAnchorElement, HtmlElement, HtmlInputElement, HtmlSelectElement,
    HtmlTextAreaElement, KeyboardEvent, SvgElement, Url,
};
use yew::prelude::*;

use crate::{
    api::{QueryResponse, query},
    model::{Filter, Metadata, Overlays, Range, TraceEvent, TrackMode, filter_sql, where_sql},
    timeline::{Overview, Selection, Timeline},
};

const DEFAULT_SQL: &str = "SELECT category, name, COUNT(*) AS count, ROUND(SUM(dur) * 1000, 3) AS total_ms\nFROM events\nGROUP BY category, name\nORDER BY total_ms DESC\nLIMIT 50";
const WORKSPACE_KEY: &str = "kutrace-workspace";
const TIMELINE_BINS: usize = 96;
const TIMELINE_DETAIL_GLYPH_BUDGET: usize = 1_000;

#[derive(Clone, Debug, PartialEq)]
struct TimelineCache {
    coverage: Range,
    filters: Vec<Filter>,
    detail: bool,
    mode: TrackMode,
}

fn prefetched_range(range: Range, full: Range) -> Range {
    let margin = range.span() * 2.0;
    Range {
        start: range.start - margin,
        end: range.end + margin,
    }
    .bounded(full)
}

fn range_contains(outer: Range, inner: Range) -> bool {
    outer.start <= inner.start && outer.end >= inner.end
}

fn density_sql(filters: &[Filter], coverage: Range, track: &str) -> String {
    let bucket = coverage.span() / TIMELINE_BINS as f64;
    let common = format!(
        "ts < {} AND ts_end > {} AND dur>0",
        coverage.end, coverage.start
    );
    let (track, scope, cpu, pid, rpc, label) = match track {
        "cpu" => (
            "cpu",
            format!("{common} AND pid>0 AND cpu>=0"),
            "track",
            "0",
            "0",
            "'cpu:' || track",
        ),
        "pid" => (
            "pid",
            format!("{common} AND pid>0 AND cpu>=0"),
            "-1",
            "track",
            "0",
            "'pid:' || track",
        ),
        "rpc" => (
            "rpc",
            format!("{common} AND rpc>0"),
            "-1",
            "0",
            "track",
            "'rpc:' || track",
        ),
        "resource" => (
            "arg0",
            format!("{common} AND category='resource' AND arg0>=0"),
            "-1",
            "0",
            "0",
            "'resource:' || track",
        ),
        _ => unreachable!("unsupported density track"),
    };
    format!(
        r#"WITH RECURSIVE scoped(first_bin,last_bin,track,event,name,category,ipc,ts,ts_end) AS (
           SELECT MIN({bins}-1,MAX(0,CAST((ts-{start})/{bucket} AS INTEGER))),
                  MIN({bins}-1,MAX(0,CAST(((ts_end-0.000000000001)-{start})/{bucket} AS INTEGER))),
                  {track},event,name,category,ipc,ts,ts_end
             FROM events {where_clause}),
         expanded(bin,last_bin,track,event,name,category,ipc,ts,ts_end) AS (
           SELECT first_bin,last_bin,track,event,name,category,ipc,ts,ts_end FROM scoped
           UNION ALL
           SELECT bin+1,last_bin,track,event,name,category,ipc,ts,ts_end
             FROM expanded WHERE bin<last_bin),
         grouped AS (
           SELECT bin,track,event,name,category,ipc,
                  SUM(MAX(0,MIN(ts_end,{start}+(bin+1)*{bucket})-MAX(ts,{start}+bin*{bucket}))) AS weight
             FROM expanded
            GROUP BY bin,track,event,name,category,ipc),
         ranked AS (
           SELECT *,ROW_NUMBER() OVER(PARTITION BY bin,track ORDER BY weight DESC,event) AS rank
             FROM grouped)
         SELECT -(bin*100000+ABS(track)+1),
                {start}+bin*{bucket},{bucket},MIN({end},{start}+(bin+1)*{bucket}),
                {cpu},{pid},{rpc},event,name,category,track,0,ipc,{label}
           FROM ranked WHERE rank=1 ORDER BY bin,track LIMIT 10000"#,
        bins = TIMELINE_BINS,
        start = coverage.start,
        end = coverage.end,
        bucket = bucket,
        track = track,
        where_clause = where_sql(filters, &scope),
        cpu = cpu,
        pid = pid,
        rpc = rpc,
        label = label,
    )
}

fn mipmap_density_sql(filters: &[Filter], coverage: Range) -> String {
    let bucket = coverage.span() / TIMELINE_BINS as f64;
    let table = if bucket < 0.016 {
        "timeline_mipmap"
    } else {
        "timeline_mipmap_coarse"
    };
    let filter = filter_sql(filters);
    let filter = if filter.is_empty() {
        String::new()
    } else {
        format!(" AND {filter}")
    };
    format!(
        r#"WITH RECURSIVE scoped(first_bin,last_bin,track,event,name,category,ipc,bucket_start,bucket_end,weight) AS (
           SELECT MIN({bins}-1,MAX(0,CAST((bucket_start-{start})/{bucket} AS INTEGER))),
                  MIN({bins}-1,MAX(0,CAST(((bucket_end-0.000000000001)-{start})/{bucket} AS INTEGER))),
                  cpu,event,name,category,ipc,bucket_start,bucket_end,weight
             FROM {table}
            WHERE bucket_start < {end} AND bucket_end > {start} AND cpu>=0{filter}),
         expanded(bin,last_bin,track,event,name,category,ipc,bucket_start,bucket_end,weight) AS (
           SELECT first_bin,last_bin,track,event,name,category,ipc,bucket_start,bucket_end,weight FROM scoped
           UNION ALL
           SELECT bin+1,last_bin,track,event,name,category,ipc,bucket_start,bucket_end,weight
             FROM expanded WHERE bin<last_bin),
         grouped AS (
           SELECT bin,track,event,name,category,ipc,
                  SUM(weight*MAX(0,MIN(bucket_end,{start}+(bin+1)*{bucket})-MAX(bucket_start,{start}+bin*{bucket}))/MAX(0.000000000001,bucket_end-bucket_start)) AS weight
             FROM expanded
            GROUP BY bin,track,event,name,category,ipc),
         ranked AS (
           SELECT *,ROW_NUMBER() OVER(PARTITION BY bin,track ORDER BY weight DESC,event) AS rank
             FROM grouped)
         SELECT -(bin*100000+ABS(track)+1),
                {start}+bin*{bucket},{bucket},MIN({end},{start}+(bin+1)*{bucket}),
                track,0,0,event,name,category,0,0,ipc,'cpu:' || track
           FROM ranked WHERE rank=1 ORDER BY bin,track LIMIT 10000"#,
        bins = TIMELINE_BINS,
        start = coverage.start,
        end = coverage.end,
        bucket = bucket,
        table = table,
        filter = filter,
    )
}

fn overlay_sql(filters: &[Filter], coverage: Range) -> String {
    let scope = format!(
        "ts < {} AND ts_end > {} AND (category IN ('mark','annotation','rpc','wakeup','lock','sample') OR event IN (521,540))",
        coverage.end, coverage.start
    );
    format!(
        "SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc,NULL \
           FROM events {} ORDER BY ts LIMIT 2001",
        where_sql(filters, &scope)
    )
}

fn long_cpu_event_sql(filters: &[Filter], coverage: Range) -> String {
    let scope = format!(
        "ts < {} AND ts_end > {} AND cpu>=0 AND (dur=0 OR dur>{})",
        coverage.end,
        coverage.start,
        0.001 * 256.0
    );
    format!(
        "SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc,'cpu:' || cpu \
           FROM events {} ORDER BY ts LIMIT 4001",
        where_sql(filters, &scope)
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SavedView {
    name: String,
    sql: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceFile {
    kind: String,
    version: u8,
    #[serde(default)]
    filters: Vec<Filter>,
    #[serde(default)]
    sql: String,
    range: Option<Range>,
    #[serde(default)]
    views: Vec<SavedView>,
    #[serde(default)]
    track_mode: TrackMode,
    #[serde(default)]
    overlays: Overlays,
}

fn value_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}

#[derive(Properties, PartialEq)]
struct TableProps {
    response: Option<QueryResponse>,
    #[prop_or_default]
    id: AttrValue,
}

#[function_component(DataTable)]
fn data_table(props: &TableProps) -> Html {
    let Some(response) = &props.response else {
        return html! {<table id={props.id.clone()}></table>};
    };
    html! {
      <table id={props.id.clone()}>
        <thead><tr>{for response.columns.iter().map(|column| html!{<th>{column}</th>})}</tr></thead>
        <tbody>{for response.rows.iter().map(|row| html!{<tr>{for row.iter().map(|value| html!{<td>{value_string(value)}</td>})}</tr>})}</tbody>
      </table>
    }
}

fn range_label(range: Range) -> String {
    format!(
        "{:.6}s – {:.6}s · {:.6}s",
        range.start,
        range.end,
        range.span()
    )
}

fn range_action(range: Range, full: Range, action: &str) -> Range {
    let center = (range.start + range.end) / 2.0;
    match action {
        "zoom-in" => Range {
            start: center - range.span() / 4.0,
            end: center + range.span() / 4.0,
        }
        .bounded(full),
        "zoom-out" => Range {
            start: center - range.span(),
            end: center + range.span(),
        }
        .bounded(full),
        "pan-left" => Range {
            start: range.start - range.span() / 4.0,
            end: range.end - range.span() / 4.0,
        }
        .bounded(full),
        "pan-right" => Range {
            start: range.start + range.span() / 4.0,
            end: range.end + range.span() / 4.0,
        }
        .bounded(full),
        _ => full,
    }
}

fn overlay_value(overlays: Overlays, name: &str) -> bool {
    match name {
        "marks" => overlays.marks,
        "arcs" => overlays.arcs,
        "locks" => overlays.locks,
        "frequency" => overlays.frequency,
        "ipc" => overlays.ipc,
        "samples" => overlays.samples,
        "colorblind" => overlays.colorblind,
        _ => false,
    }
}

fn toggle_overlay(mut overlays: Overlays, name: &str) -> Overlays {
    match name {
        "marks" => overlays.marks = !overlays.marks,
        "arcs" => overlays.arcs = !overlays.arcs,
        "locks" => overlays.locks = !overlays.locks,
        "frequency" => overlays.frequency = !overlays.frequency,
        "ipc" => overlays.ipc = !overlays.ipc,
        "samples" => overlays.samples = !overlays.samples,
        "colorblind" => overlays.colorblind = !overlays.colorblind,
        _ => {}
    }
    overlays
}

#[derive(Clone, Debug, PartialEq)]
struct ProfileFrame {
    sample_id: i64,
    depth: usize,
    name: String,
    has_callchain: bool,
}

#[derive(Default)]
struct FlameNode {
    name: String,
    samples: usize,
    children: BTreeMap<String, FlameNode>,
}

#[derive(Clone)]
struct FlameRect {
    name: String,
    depth: usize,
    left: f64,
    width: f64,
    samples: usize,
}

fn layout_flame_node(
    node: &FlameNode,
    depth: usize,
    left: f64,
    width: f64,
    output: &mut Vec<FlameRect>,
) {
    let mut cursor = left;
    for child in node.children.values() {
        let child_width = if node.samples == 0 {
            0.0
        } else {
            width * child.samples as f64 / node.samples as f64
        };
        if child_width > 0.0 {
            output.push(FlameRect {
                name: child.name.clone(),
                depth,
                left: cursor,
                width: child_width,
                samples: child.samples,
            });
            layout_flame_node(child, depth + 1, cursor, child_width, output);
            cursor += child_width;
        }
    }
}

fn callchain_flame(frames: &[ProfileFrame]) -> (Vec<FlameRect>, usize, usize) {
    let mut samples = BTreeMap::<i64, Vec<&ProfileFrame>>::new();
    for frame in frames.iter().filter(|frame| frame.has_callchain) {
        samples.entry(frame.sample_id).or_default().push(frame);
    }
    let mut root = FlameNode {
        name: "root".to_owned(),
        samples: samples.len(),
        children: BTreeMap::new(),
    };
    for sample in samples.values_mut() {
        sample.sort_by_key(|frame| frame.depth);
        let mut node = &mut root;
        for frame in sample.iter() {
            node = node
                .children
                .entry(frame.name.clone())
                .or_insert_with(|| FlameNode {
                    name: frame.name.clone(),
                    ..FlameNode::default()
                });
            node.samples += 1;
        }
    }
    let mut output = Vec::new();
    layout_flame_node(&root, 1, 0.0, 100.0, &mut output);
    let max_depth = output.iter().map(|frame| frame.depth).max().unwrap_or(0);
    (output, root.samples, max_depth)
}

fn agent_depth(event: &TraceEvent, parents: &HashMap<i64, i64>) -> usize {
    let mut depth = 0;
    let mut parent = event.retval;
    let mut visited = HashSet::new();
    while parent > 0 && depth < 8 && visited.insert(parent) {
        depth += 1;
        parent = parents.get(&parent).copied().unwrap_or_default();
    }
    depth
}

#[function_component(App)]
pub fn app() -> Html {
    let metadata = use_state(Metadata::default);
    let range = use_state(Range::default);
    let events = use_state(|| Rc::new(Vec::<TraceEvent>::new()));
    let timeline_loading = use_state(|| true);
    let timeline_truncated = use_state(|| false);
    let timeline_source = use_state(|| "events".to_owned());
    let timeline_cache = use_state(|| None::<TimelineCache>);
    let timeline_generation = use_mut_ref(|| 0_u64);
    let error = use_state(String::new);
    let filters = use_state(Vec::<Filter>::new);
    let filter_field = use_state(|| "category".to_owned());
    let filter_op = use_state(|| "=".to_owned());
    let filter_value = use_state(String::new);
    let track_mode = use_state(TrackMode::default);
    let overlays = use_state(Overlays::default);
    let highlighted = use_state(HashSet::<String>::new);
    let search = use_state(String::new);
    let search_invert = use_state(|| false);
    let selection = use_state(|| None::<Selection>);
    let active_view = use_state(|| "timeline".to_owned());
    let active_dock = use_state(|| "details".to_owned());
    let dock_open = use_state(|| false);
    let sql = use_state(|| DEFAULT_SQL.to_owned());
    let sql_result = use_state(|| None::<QueryResponse>);
    let sql_status = use_state(String::new);
    let flame_weight = use_state(|| "duration".to_owned());
    let legacy_loaded = use_state(|| false);
    let profile_frames = use_state(Vec::<ProfileFrame>::new);
    let profile_status = use_state(String::new);
    let saved_views = use_state(Vec::<SavedView>::new);
    let view_name = use_state(String::new);
    let workspace_loaded = use_state(|| false);
    let follow_tail = use_state(|| false);
    let navigation_keys = use_state(HashSet::<String>::new);

    {
        let metadata = metadata.clone();
        let range = range.clone();
        let error = error.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                let result = async {
                    let meta = query(
                        "SELECT key,value FROM metadata WHERE key IN ('title','tracebase','flags')",
                        10,
                    )
                    .await?;
                    let extent =
                        query("SELECT COUNT(*),MIN(ts),MAX(ts_end) FROM events", 1).await?;
                    let values = meta
                        .rows
                        .iter()
                        .filter_map(|row| {
                            Some((
                                row.first()?.as_str()?.to_owned(),
                                row.get(1)?.as_str()?.to_owned(),
                            ))
                        })
                        .collect::<BTreeMap<_, _>>();
                    let row = extent.rows.first().cloned().unwrap_or_default();
                    let start = row.get(1).and_then(Value::as_f64).unwrap_or_default();
                    let mut end = row
                        .get(2)
                        .and_then(Value::as_f64)
                        .unwrap_or(start + 0.000_001);
                    if end <= start {
                        end = start + 0.000_001;
                    }
                    Ok::<_, String>(Metadata {
                        title: [values.get("title"), values.get("tracebase")]
                            .into_iter()
                            .flatten()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" · "),
                        count: row.first().and_then(Value::as_u64).unwrap_or_default(),
                        flags: values
                            .get("flags")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or_default(),
                        full: Range { start, end },
                    })
                }
                .await;
                match result {
                    Ok(value) => {
                        range.set(value.full);
                        metadata.set(value);
                    }
                    Err(message) => error.set(message),
                }
            });
            || ()
        });
    }

    {
        let filters = filters.clone();
        let sql = sql.clone();
        let range = range.clone();
        let saved_views = saved_views.clone();
        let track_mode = track_mode.clone();
        let overlays = overlays.clone();
        let workspace_loaded = workspace_loaded.clone();
        let status = sql_status.clone();
        let full = metadata.full;
        use_effect_with(full, move |full| {
            if full.end > full.start && !*workspace_loaded {
                workspace_loaded.set(true);
                let stored = web_sys::window()
                    .and_then(|window| window.local_storage().ok().flatten())
                    .and_then(|storage| storage.get_item(WORKSPACE_KEY).ok().flatten());
                if let Some(stored) = stored {
                    match serde_json::from_str::<WorkspaceFile>(&stored) {
                        Ok(workspace)
                            if workspace.kind == "kutrace-workspace"
                                && matches!(workspace.version, 1 | 2) =>
                        {
                            filters.set(workspace.filters.into_iter().take(64).collect());
                            if !workspace.sql.is_empty() {
                                sql.set(workspace.sql);
                            }
                            if let Some(saved_range) = workspace.range {
                                range.set(saved_range.bounded(*full));
                            }
                            saved_views.set(workspace.views.into_iter().take(32).collect());
                            track_mode.set(workspace.track_mode);
                            overlays.set(workspace.overlays);
                        }
                        Ok(_) => status.set("Unsupported saved workspace".to_owned()),
                        Err(error) => status.set(format!("Invalid saved workspace: {error}")),
                    }
                }
            }
            || ()
        });
    }

    {
        let metadata_handle = metadata.clone();
        let range_handle = range.clone();
        let status = sql_status.clone();
        let current_metadata = (*metadata).clone();
        let current_range = *range;
        let following = *follow_tail;
        use_effect_with(
            (
                following,
                current_metadata.count,
                current_metadata.full,
                current_range,
            ),
            move |_| {
                let interval = Interval::new(1_500, move || {
                    let metadata_handle = metadata_handle.clone();
                    let range_handle = range_handle.clone();
                    let status = status.clone();
                    let current_metadata = current_metadata.clone();
                    spawn_local(async move {
                        match query("SELECT COUNT(*),MIN(ts),MAX(ts_end) FROM events", 1).await {
                            Ok(response) => {
                                let row = response.rows.first().cloned().unwrap_or_default();
                                let count = row.first().and_then(Value::as_u64).unwrap_or_default();
                                let start = row
                                    .get(1)
                                    .and_then(Value::as_f64)
                                    .unwrap_or(current_metadata.full.start);
                                let end = row
                                    .get(2)
                                    .and_then(Value::as_f64)
                                    .unwrap_or(current_metadata.full.end)
                                    .max(start + f64::EPSILON);
                                if count != current_metadata.count
                                    || end != current_metadata.full.end
                                {
                                    let full = Range { start, end };
                                    let mut next = current_metadata.clone();
                                    next.count = count;
                                    next.full = full;
                                    metadata_handle.set(next);
                                    if following {
                                        range_handle.set(
                                            Range {
                                                start: end - current_range.span(),
                                                end,
                                            }
                                            .bounded(full),
                                        );
                                    }
                                }
                            }
                            Err(message) => status.set(message),
                        }
                    });
                });
                move || drop(interval)
            },
        );
    }

    {
        let profile_frames = profile_frames.clone();
        let profile_status = profile_status.clone();
        let profile_range = selection
            .as_ref()
            .map(|selection| selection.range)
            .unwrap_or(*range);
        let flame_visible = *dock_open && *active_dock == "flamegraph";
        use_effect_with(
            (profile_range, flame_visible),
            move |(profile_range, visible)| {
                if *visible && profile_range.end > profile_range.start {
                    let profile_frames = profile_frames.clone();
                    let profile_status = profile_status.clone();
                    let profile_range = *profile_range;
                    spawn_local(async move {
                        let statement = format!(
                            "SELECT sample_id,depth,name,has_callchain FROM profile_callchains WHERE ts>={} AND ts<{} ORDER BY sample_id,depth LIMIT 10000",
                            profile_range.start, profile_range.end
                        );
                        match query(&statement, 10_000).await {
                            Ok(response) => {
                                let frames = response
                                    .rows
                                    .iter()
                                    .map(|row| ProfileFrame {
                                        sample_id: row
                                            .first()
                                            .and_then(Value::as_i64)
                                            .unwrap_or_default(),
                                        depth: row
                                            .get(1)
                                            .and_then(Value::as_u64)
                                            .unwrap_or_default()
                                            as usize,
                                        name: row
                                            .get(2)
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .to_owned(),
                                        has_callchain: row
                                            .get(3)
                                            .and_then(Value::as_i64)
                                            .unwrap_or_default()
                                            != 0,
                                    })
                                    .collect::<Vec<_>>();
                                let sample_count = frames
                                    .iter()
                                    .map(|frame| frame.sample_id)
                                    .collect::<HashSet<_>>()
                                    .len();
                                profile_status.set(if response.truncated {
                                    format!("{sample_count}+ sampled stacks")
                                } else {
                                    format!("{sample_count} sampled stacks")
                                });
                                profile_frames.set(frames);
                            }
                            Err(message) => profile_status.set(message),
                        }
                    });
                }
                || ()
            },
        );
    }

    {
        let events = events.clone();
        let loading = timeline_loading.clone();
        let truncated = timeline_truncated.clone();
        let source = timeline_source.clone();
        let cache = timeline_cache.clone();
        let generation = timeline_generation.clone();
        let error = error.clone();
        let current_range = *range;
        let current_filters = (*filters).clone();
        let current_cache = (*timeline_cache).clone();
        let current_mode = *track_mode;
        let full = metadata.full;
        use_effect_with(
            (
                current_range,
                current_filters.clone(),
                current_cache,
                current_mode,
                full,
            ),
            move |(current_range, current_filters, current_cache, current_mode, full)| {
                *generation.borrow_mut() += 1;
                let request_generation = *generation.borrow();
                let reusable = current_cache.as_ref().is_some_and(|cached| {
                    cached.filters == *current_filters
                        && cached.mode == *current_mode
                        && range_contains(cached.coverage, *current_range)
                        && (cached.detail || current_range.span() >= cached.coverage.span() / 5.0)
                });
                let timeout = if reusable {
                    loading.set(false);
                    None
                } else if current_range.end > current_range.start {
                    let events = events.clone();
                    let loading = loading.clone();
                    let truncated = truncated.clone();
                    let source = source.clone();
                    let cache = cache.clone();
                    let generation = generation.clone();
                    let error = error.clone();
                    let current_range = *current_range;
                    let current_filters = current_filters.clone();
                    let current_mode = *current_mode;
                    let coverage = prefetched_range(current_range, *full);
                    Some(Timeout::new(70, move || {
                        loading.set(true);
                        spawn_local(async move {
                            let scope =
                                format!("ts < {} AND ts_end > {}", coverage.end, coverage.start);
                            let sql = format!(
                                "SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc FROM events {} ORDER BY ts,dur DESC LIMIT 10001",
                                where_sql(&current_filters, &scope)
                            );
                            let result = async {
                                let response = query(&sql, 10_000).await?;
                                let projected_glyphs = response.rows.len()
                                    * if matches!(current_mode, TrackMode::CpuPid) {
                                        4
                                    } else {
                                        1
                                    };
                                let use_density = response.truncated
                                    || response.rows.len() > 10_000
                                    || projected_glyphs > TIMELINE_DETAIL_GLYPH_BUDGET;
                                if !use_density {
                                    return Ok::<_, String>((
                                        response
                                            .rows
                                            .iter()
                                            .map(|row| TraceEvent::from_row(row))
                                            .collect::<Vec<_>>(),
                                        false,
                                        "events".to_owned(),
                                    ));
                                }
                                let mmap_compatible = current_filters.iter().all(|filter| {
                                    matches!(filter.field.as_str(), "category" | "cpu" | "event")
                                });
                                let mut rows = Vec::new();
                                let mut used_mipmap = false;
                                let mut partial = false;
                                if !matches!(current_mode, TrackMode::Pid) {
                                    let sql = if mmap_compatible {
                                        used_mipmap = true;
                                        mipmap_density_sql(&current_filters, coverage)
                                    } else {
                                        density_sql(&current_filters, coverage, "cpu")
                                    };
                                    let response = query(&sql, 10_000).await?;
                                    rows.extend(
                                        response.rows.iter().map(|row| TraceEvent::from_row(row)),
                                    );
                                    partial |= response.truncated;
                                    if used_mipmap {
                                        let long_events = query(
                                            &long_cpu_event_sql(&current_filters, coverage),
                                            4_000,
                                        )
                                        .await?;
                                        partial |=
                                            long_events.truncated || long_events.rows.len() > 4_000;
                                        rows.extend(
                                            long_events
                                                .rows
                                                .iter()
                                                .map(|row| TraceEvent::from_row(row)),
                                        );
                                    }
                                }
                                if !matches!(current_mode, TrackMode::Cpu) {
                                    let response = query(
                                        &density_sql(&current_filters, coverage, "pid"),
                                        10_000,
                                    )
                                    .await?;
                                    rows.extend(
                                        response.rows.iter().map(|row| TraceEvent::from_row(row)),
                                    );
                                    partial |= response.truncated;
                                }
                                if matches!(current_mode, TrackMode::CpuPid) {
                                    for semantic_track in ["rpc", "resource"] {
                                        let response = query(
                                            &density_sql(
                                                &current_filters,
                                                coverage,
                                                semantic_track,
                                            ),
                                            10_000,
                                        )
                                        .await?;
                                        rows.extend(
                                            response
                                                .rows
                                                .iter()
                                                .map(|row| TraceEvent::from_row(row)),
                                        );
                                        partial |= response.truncated;
                                    }
                                }
                                let overlays =
                                    query(&overlay_sql(&current_filters, coverage), 2_000).await?;
                                rows.extend(
                                    overlays.rows.iter().map(|row| TraceEvent::from_row(row)),
                                );
                                let mut source =
                                    if used_mipmap && matches!(current_mode, TrackMode::Cpu) {
                                        "mipmap".to_owned()
                                    } else {
                                        "summary".to_owned()
                                    };
                                partial |= overlays.truncated || overlays.rows.len() > 2_000;
                                if partial {
                                    source.push_str("-partial");
                                }
                                Ok((rows, true, source))
                            }
                            .await;
                            if *generation.borrow() != request_generation {
                                return;
                            }
                            match result {
                                Ok((rows, is_truncated, data_source)) => {
                                    events.set(Rc::new(rows));
                                    truncated.set(is_truncated);
                                    source.set(data_source);
                                    cache.set(Some(TimelineCache {
                                        coverage,
                                        filters: current_filters,
                                        detail: !is_truncated,
                                        mode: current_mode,
                                    }));
                                    error.set(String::new());
                                }
                                Err(message) => error.set(message),
                            }
                            loading.set(false);
                        });
                    }))
                } else {
                    None
                };
                move || drop(timeout)
            },
        );
    }

    {
        let sql = (*sql).clone();
        let result = sql_result.clone();
        let status = sql_status.clone();
        use_effect_with((), move |_| {
            status.set("Running…".to_owned());
            spawn_local(async move {
                match query(&sql, 1_000).await {
                    Ok(response) => {
                        status.set(format!(
                            "{}{} rows · {:.2} ms",
                            response.rows.len(),
                            if response.truncated {
                                " (truncated)"
                            } else {
                                ""
                            },
                            response.elapsed_ms
                        ));
                        result.set(Some(response));
                    }
                    Err(message) => status.set(message),
                }
            });
            || ()
        });
    }

    {
        let range_handle = range.clone();
        let active_view = active_view.clone();
        let navigation_keys = navigation_keys.clone();
        let current_range = *range;
        let full = metadata.full;
        use_effect_with(
            (current_range, full, (*active_view).clone()),
            move |(current_range, full, view)| {
                let current_range = *current_range;
                let full = *full;
                let view = view.clone();
                let range_handle = range_handle.clone();
                let navigation_keys_down = navigation_keys.clone();
                let keydown =
                    EventListener::new(&web_sys::window().unwrap(), "keydown", move |event| {
                        let Some(keyboard) = event.dyn_ref::<KeyboardEvent>() else {
                            return;
                        };
                        let target_is_input = keyboard
                            .target()
                            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                            .is_some_and(|target| {
                                target
                                    .matches("input,textarea,select,[contenteditable=true]")
                                    .unwrap_or(false)
                            });
                        if keyboard.key() == "Escape" && target_is_input {
                            keyboard.prevent_default();
                            if let Some(target) = keyboard
                                .target()
                                .and_then(|target| target.dyn_into::<HtmlElement>().ok())
                            {
                                target.blur().ok();
                            }
                            if view == "timeline" {
                                if let Some(timeline) = web_sys::window()
                                    .and_then(|window| window.document())
                                    .and_then(|document| document.get_element_by_id("timeline"))
                                    .and_then(|element| element.dyn_into::<SvgElement>().ok())
                                {
                                    timeline.focus().ok();
                                }
                            }
                            return;
                        }
                        if target_is_input
                            || keyboard.ctrl_key()
                            || keyboard.meta_key()
                            || keyboard.alt_key()
                            || view != "timeline"
                        {
                            return;
                        }
                        let action = match keyboard.key().as_str() {
                            "w" | "W" | "+" | "=" => "zoom-in",
                            "s" | "S" | "-" => "zoom-out",
                            "a" | "A" | "ArrowLeft" | "[" => "pan-left",
                            "d" | "D" | "ArrowRight" | "]" => "pan-right",
                            "0" | "Home" => "reset",
                            _ => return,
                        };
                        keyboard.prevent_default();
                        let normalized = keyboard.key().to_lowercase();
                        if matches!(normalized.as_str(), "w" | "a" | "s" | "d") {
                            let mut next = (*navigation_keys_down).clone();
                            next.insert(normalized);
                            navigation_keys_down.set(next);
                        } else {
                            range_handle.set(range_action(current_range, full, action));
                        }
                    });
                let navigation_keys_up = navigation_keys.clone();
                let keyup =
                    EventListener::new(&web_sys::window().unwrap(), "keyup", move |event| {
                        let Some(keyboard) = event.dyn_ref::<KeyboardEvent>() else {
                            return;
                        };
                        let normalized = keyboard.key().to_lowercase();
                        if matches!(normalized.as_str(), "w" | "a" | "s" | "d") {
                            let mut next = (*navigation_keys_up).clone();
                            next.remove(&normalized);
                            navigation_keys_up.set(next);
                        }
                    });
                move || {
                    drop(keydown);
                    drop(keyup);
                }
            },
        );
    }

    {
        let range = range.clone();
        let keys = (*navigation_keys).clone();
        let full = metadata.full;
        use_effect_with((keys.clone(), full), move |(keys, full)| {
            let interval = if keys.is_empty() {
                None
            } else {
                let range = range.clone();
                let keys = keys.clone();
                let full = *full;
                Some(Interval::new(16, move || {
                    let current = *range;
                    let mut span = current.span();
                    let mut center = (current.start + current.end) / 2.0;
                    let zoom_in = keys.contains("w");
                    let zoom_out = keys.contains("s");
                    if zoom_in != zoom_out {
                        span *= ((if zoom_out { 1.0 } else { -1.0 }) * 1.8_f64 / 60.0).exp();
                    }
                    let left = keys.contains("a");
                    let right = keys.contains("d");
                    if left != right {
                        center += (if right { 1.0 } else { -1.0 }) * span * 0.72 / 60.0;
                    }
                    range.set(
                        Range {
                            start: center - span / 2.0,
                            end: center + span / 2.0,
                        }
                        .bounded(full),
                    );
                }))
            };
            move || drop(interval)
        });
    }

    let set_range = {
        let range = range.clone();
        Callback::from(move |next: Range| range.set(next))
    };
    let navigate = |action: &'static str| {
        let range = range.clone();
        let current = *range;
        let full = metadata.full;
        Callback::from(move |_| range.set(range_action(current, full, action)))
    };
    let on_select = {
        let selection = selection.clone();
        Callback::from(move |next| selection.set(next))
    };
    let on_highlight = {
        let highlighted = highlighted.clone();
        Callback::from(move |track: String| {
            let mut next = (*highlighted).clone();
            if !next.remove(&track) {
                next.insert(track);
            }
            highlighted.set(next);
        })
    };
    let add_filter = {
        let filters = filters.clone();
        let value = filter_value.clone();
        let field = filter_field.clone();
        let op = filter_op.clone();
        Callback::from(move |_| {
            let entered = value.trim().to_owned();
            if entered.is_empty() {
                return;
            }
            let mut next = (*filters).clone();
            next.push(Filter {
                field: (*field).clone(),
                op: (*op).clone(),
                value: entered,
            });
            filters.set(next);
            value.set(String::new());
        })
    };
    let run_sql = {
        let sql = sql.clone();
        let result = sql_result.clone();
        let status = sql_status.clone();
        Callback::from(move |_| {
            let statement = (*sql).clone();
            let result = result.clone();
            let status = status.clone();
            status.set("Running…".to_owned());
            spawn_local(async move {
                match query(&statement, 1_000).await {
                    Ok(response) => {
                        status.set(format!(
                            "{}{} rows · {:.2} ms",
                            response.rows.len(),
                            if response.truncated {
                                " (truncated)"
                            } else {
                                ""
                            },
                            response.elapsed_ms
                        ));
                        result.set(Some(response));
                    }
                    Err(message) => status.set(message),
                }
            });
        })
    };
    let workspace = WorkspaceFile {
        kind: "kutrace-workspace".to_owned(),
        version: 2,
        filters: (*filters).clone(),
        sql: (*sql).clone(),
        range: Some(*range),
        views: (*saved_views).clone(),
        track_mode: *track_mode,
        overlays: *overlays,
    };
    let save_workspace = {
        let status = sql_status.clone();
        let serialized = serde_json::to_string(&workspace).unwrap_or_default();
        Callback::from(move |_| {
            let outcome = web_sys::window()
                .and_then(|window| window.local_storage().ok().flatten())
                .ok_or_else(|| "Local storage is unavailable".to_owned())
                .and_then(|storage| {
                    storage
                        .set_item(WORKSPACE_KEY, &serialized)
                        .map_err(|_| "Could not save workspace".to_owned())
                });
            status.set(
                outcome
                    .map(|_| "Workspace saved locally".to_owned())
                    .unwrap_or_else(|message| message),
            );
        })
    };
    let export_workspace = {
        let status = sql_status.clone();
        let serialized = serde_json::to_string_pretty(&workspace).unwrap_or_default();
        Callback::from(move |_| {
            let outcome = (|| {
                let values = js_sys::Array::new();
                values.push(&JsValue::from_str(&serialized));
                let options = BlobPropertyBag::new();
                options.set_type("application/json");
                let blob = Blob::new_with_str_sequence_and_options(&values, &options)
                    .map_err(|_| "Could not create workspace export")?;
                let url = Url::create_object_url_with_blob(&blob)
                    .map_err(|_| "Could not create workspace URL")?;
                let document = web_sys::window()
                    .and_then(|window| window.document())
                    .ok_or("Document unavailable")?;
                let anchor = document
                    .create_element("a")
                    .map_err(|_| "Could not create export link")?
                    .dyn_into::<HtmlAnchorElement>()
                    .map_err(|_| "Could not create export link")?;
                anchor.set_href(&url);
                anchor.set_download("kutrace-workspace.json");
                anchor.click();
                Url::revoke_object_url(&url).ok();
                Ok::<_, &str>(())
            })();
            status.set(
                outcome
                    .map(|_| "Workspace exported".to_owned())
                    .unwrap_or_else(str::to_owned),
            );
        })
    };
    let import_workspace = Callback::from(move |_| {
        if let Some(input) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("workspace-file"))
            .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
        {
            input.click();
        }
    });
    let on_workspace_file = {
        let filters = filters.clone();
        let sql = sql.clone();
        let range = range.clone();
        let saved_views = saved_views.clone();
        let track_mode = track_mode.clone();
        let overlays = overlays.clone();
        let status = sql_status.clone();
        let full = metadata.full;
        Callback::from(move |event: Event| {
            let input = event.target_unchecked_into::<HtmlInputElement>();
            let Some(file) = input.files().and_then(|files| files.get(0)) else {
                return;
            };
            let filters = filters.clone();
            let sql = sql.clone();
            let range = range.clone();
            let saved_views = saved_views.clone();
            let track_mode = track_mode.clone();
            let overlays = overlays.clone();
            let status = status.clone();
            spawn_local(async move {
                let outcome = async {
                    let text = JsFuture::from(file.text())
                        .await
                        .map_err(|_| "Could not read workspace file".to_owned())?
                        .as_string()
                        .ok_or_else(|| "Workspace file is not text".to_owned())?;
                    let workspace = serde_json::from_str::<WorkspaceFile>(&text)
                        .map_err(|error| format!("Invalid workspace: {error}"))?;
                    if workspace.kind != "kutrace-workspace" || !matches!(workspace.version, 1 | 2)
                    {
                        return Err("Unsupported workspace file".to_owned());
                    }
                    if workspace.filters.len() > 64 || workspace.views.len() > 32 {
                        return Err("Workspace exceeds bounded filter/view limits".to_owned());
                    }
                    filters.set(workspace.filters);
                    if !workspace.sql.is_empty() {
                        sql.set(workspace.sql);
                    }
                    if let Some(saved_range) = workspace.range {
                        range.set(saved_range.bounded(full));
                    }
                    saved_views.set(workspace.views);
                    track_mode.set(workspace.track_mode);
                    overlays.set(workspace.overlays);
                    Ok::<_, String>(())
                }
                .await;
                status.set(
                    outcome
                        .map(|_| "Workspace imported".to_owned())
                        .unwrap_or_else(|message| message),
                );
            });
            input.set_value("");
        })
    };
    let save_view = {
        let saved_views = saved_views.clone();
        let view_name = view_name.clone();
        let sql = sql.clone();
        let status = sql_status.clone();
        Callback::from(move |_| {
            let name = view_name.trim().to_owned();
            let statement = sql.trim().to_owned();
            if name.is_empty() || statement.is_empty() {
                status.set("Name the SQL view before saving".to_owned());
                return;
            }
            let mut next = (*saved_views).clone();
            if let Some(existing) = next.iter_mut().find(|view| view.name == name) {
                existing.sql = statement;
            } else if next.len() < 32 {
                next.push(SavedView {
                    name: name.clone(),
                    sql: statement,
                });
            } else {
                status.set("Saved SQL view limit is 32".to_owned());
                return;
            }
            saved_views.set(next);
            status.set(format!("Saved view “{name}”"));
        })
    };
    let show_schema = {
        let active_dock = active_dock.clone();
        let dock_open = dock_open.clone();
        let result = sql_result.clone();
        let status = sql_status.clone();
        Callback::from(move |_| {
            active_dock.set("sql".to_owned());
            dock_open.set(true);
            let result = result.clone();
            let status = status.clone();
            spawn_local(async move {
                match query("SELECT type,name,sql FROM sqlite_schema WHERE type IN ('table','view') ORDER BY type,name", 1_000).await {
                    Ok(response) => { status.set("sqlite_schema".to_owned()); result.set(Some(response)); }
                    Err(message) => status.set(message),
                }
            });
        })
    };
    let toggle_follow = {
        let follow_tail = follow_tail.clone();
        let range = range.clone();
        let full = metadata.full;
        Callback::from(move |_| {
            let next = !*follow_tail;
            follow_tail.set(next);
            if next {
                range.set(
                    Range {
                        start: full.end - range.span(),
                        end: full.end,
                    }
                    .bounded(full),
                );
            }
        })
    };

    let mut categories: BTreeMap<String, Vec<&TraceEvent>> = BTreeMap::new();
    let flame_range = selection
        .as_ref()
        .map(|selection| selection.range)
        .unwrap_or(*range);
    let render_flame = *dock_open && *active_dock == "flamegraph";
    if render_flame {
        for event in events
            .iter()
            .filter(|event| event.start < flame_range.end && event.end > flame_range.start)
        {
            categories
                .entry(event.category.clone())
                .or_default()
                .push(event);
        }
    }
    let flame_total: f64 = if *flame_weight == "duration" {
        categories
            .values()
            .flatten()
            .map(|event| event.duration.max(0.0))
            .sum()
    } else {
        categories.values().map(|events| events.len() as f64).sum()
    };
    let mut flame_offset = 0.0;
    let mut flame_frames = Vec::new();
    for (category, category_events) in &categories {
        let category_weight = if *flame_weight == "duration" {
            category_events
                .iter()
                .map(|event| event.duration.max(0.0))
                .sum()
        } else {
            category_events.len() as f64
        };
        if category_weight <= 0.0 || flame_total <= 0.0 {
            continue;
        }
        let category_width = category_weight / flame_total * 100.0;
        flame_frames.push(html!{<button class="flame-frame" data-flame-frame="true" data-flame-level="category" style={format!("left:{flame_offset}%;top:31px;width:{category_width}%;background:#d783ff")}>{category}</button>});
        let mut by_name = BTreeMap::<String, Vec<&&TraceEvent>>::new();
        for event in category_events {
            by_name
                .entry(if event.name.is_empty() {
                    "(unnamed)".to_owned()
                } else {
                    event.name.clone()
                })
                .or_default()
                .push(event);
        }
        let mut item_offset = flame_offset;
        for (name, named_events) in by_name {
            let weight = if *flame_weight == "duration" {
                named_events
                    .iter()
                    .map(|event| event.duration.max(0.0))
                    .sum()
            } else {
                named_events.len() as f64
            };
            let width = weight / flame_total * 100.0;
            flame_frames.push(html!{<button class="flame-frame" data-flame-frame="true" data-flame-level="name" style={format!("left:{item_offset}%;top:60px;width:{width}%;background:#a8e6cf")}>{name}</button>});
            item_offset += width;
        }
        flame_offset += category_width;
    }
    let (callchain_rects, callchain_samples, callchain_depth) = if render_flame {
        callchain_flame(&profile_frames)
    } else {
        (Vec::new(), 0, 0)
    };
    if callchain_samples > 0 {
        flame_frames = callchain_rects
            .iter()
            .map(|frame| {
                let hue = (frame.name.bytes().fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(u32::from(byte))) % 90) + 10;
                html! {<button class="flame-frame" data-flame-frame="true" data-flame-level="callchain" style={format!("left:{}%;top:{}px;width:{}%;background:hsl({hue} 68% 67%)",frame.left,2+frame.depth*29,frame.width)} title={format!("{} · {} samples",frame.name,frame.samples)}>{frame.name.clone()}</button>}
            })
            .collect();
    }

    let render_agent = *dock_open && *active_dock == "agent";
    let mut visible_agent_spans = if render_agent {
        events
            .iter()
            .filter(|event| {
                event.event == 645 && event.start < range.end && event.end > range.start
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    visible_agent_spans.sort_by(|left, right| left.start.total_cmp(&right.start));
    let agent_parents = visible_agent_spans
        .iter()
        .map(|event| (event.arg0, event.retval))
        .collect::<HashMap<_, _>>();
    let selected_agent = selection
        .as_ref()
        .and_then(|selected| selected.event.as_ref())
        .filter(|event| event.event == 645)
        .cloned();
    let selected_agent_id = selected_agent.as_ref().map(|event| event.arg0);
    let agent_nodes = visible_agent_spans
        .iter()
        .map(|event| {
            let event_copy = event.clone();
            let selection = selection.clone();
            let depth = agent_depth(event, &agent_parents);
            let selected = selected_agent_id == Some(event.arg0);
            html! {
              <button class={classes!("agent-node",selected.then_some("selected"))}
                data-agent-span={event.arg0.to_string()}
                aria-label={format!("Select agent span {} {}",event.arg0,event.name)}
                style={format!("padding-left:{}px",8+depth*18)}
                onclick={Callback::from(move |_| selection.set(Some(Selection {
                    range: Range { start:event_copy.start,end:event_copy.end },
                    event:Some(event_copy.clone()),
                })))}>
                <span>{if depth==0 {"◆"} else {"↳"}}</span>
                {format!(" {} · {:.3} ms",event.name,event.duration*1000.0)}
              </button>
            }
        })
        .collect::<Vec<_>>();
    let context_range = selected_agent
        .as_ref()
        .map(|event| Range {
            start: event.start,
            end: event.end,
        })
        .unwrap_or(*range);
    let related_rpcs = if render_agent {
        events
            .iter()
            .filter(|event| {
                event.rpc > 0 && event.start < context_range.end && event.end > context_range.start
            })
            .take(32)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let related_resources = if render_agent {
        events
            .iter()
            .filter(|event| {
                event.category == "resource"
                    && event.start < context_range.end
                    && event.end > context_range.start
            })
            .take(32)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let annotations = selected_agent_id
        .map(|span_id| {
            events
                .iter()
                .filter(|event| event.category == "annotation" && event.retval == span_id)
                .take(32)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let context_sql = selected_agent.as_ref().map(|event| {
        format!(
            "SELECT ts,dur,cpu,pid,rpc,event,name,category,arg0,retval,ipc\n\
             FROM events\n\
             WHERE ts < {end} AND ts_end > {start}\n\
             ORDER BY ts,dur DESC\n\
             LIMIT 1000",
            start = event.start,
            end = event.end
        )
    });
    let open_agent_context = {
        let sql = sql.clone();
        let active_dock = active_dock.clone();
        let statement = context_sql.clone();
        Callback::from(move |_| {
            if let Some(statement) = &statement {
                sql.set(statement.clone());
                active_dock.set("sql".to_owned());
            }
        })
    };

    let search_matches = if search.is_empty() {
        0
    } else {
        events
            .iter()
            .filter(|event| {
                let needle = search.to_lowercase();
                let matched = event.name.to_lowercase().contains(&needle)
                    || event.category.to_lowercase().contains(&needle)
                    || event.pid.to_string().contains(&needle)
                    || event.cpu.to_string().contains(&needle);
                if *search_invert { !matched } else { matched }
            })
            .count()
    };
    let saved_view_rows = if saved_views.is_empty() {
        html! {"No saved SQL views."}
    } else {
        html! {<> {for saved_views.iter().enumerate().map(|(index, view)| {
            let sql = sql.clone();
            let view_name = view_name.clone();
            let active_dock = active_dock.clone();
            let dock_open = dock_open.clone();
            let saved_views = saved_views.clone();
            let load = view.clone();
            html! {
              <div class="saved-view-row">
                <button data-load-view={index.to_string()} onclick={Callback::from(move |_| {
                    sql.set(load.sql.clone());
                    view_name.set(load.name.clone());
                    active_dock.set("sql".to_owned());
                    dock_open.set(true);
                })}><span>{view.name.clone()}</span></button>
                <button data-delete-view={index.to_string()} onclick={Callback::from(move |_| {
                    let mut next=(*saved_views).clone();
                    next.remove(index);
                    saved_views.set(next);
                })}>{"×"}</button>
              </div>
            }
        })} </>}
    };

    html! {
      <div class={classes!("wasm-app", overlays.colorblind.then_some("colorblind"))}>
        <section class="print-summary print-only" aria-label="Printed trace summary"><h1>{"KUtrace workspace"}</h1><p id="print-trace-title">{metadata.title.clone()}</p><p id="print-range">{format!("Visible range: {}", range_label(*range))}</p><p id="print-filters">{if filters.is_empty() {"Filters: none".to_owned()} else {format!("Filters: {}", filters.iter().map(|filter| format!("{} {} {}", filter.field, filter.op, filter.value)).collect::<Vec<_>>().join(" · "))}}</p></section>
        <header class="app-header"><div class="brand"><span class="mark">{"KU"}</span><strong>{"trace"}</strong><span id="trace-title">{metadata.title.clone()}</span></div><nav><span id="live-status" class="live-status">{format!("{} events · {}", metadata.count,if *follow_tail{"following live"}else{"static"})}</span><button id="follow-live" aria-pressed={follow_tail.to_string()} onclick={toggle_follow}>{"Follow tail"}</button><button id="save-workspace" onclick={save_workspace}>{"Save"}</button><button id="export-workspace" onclick={export_workspace}>{"Export"}</button><button id="import-workspace" onclick={import_workspace}>{"Import"}</button><input id="workspace-file" type="file" accept="application/json,.json" onchange={on_workspace_file} hidden=true/><a href="/legacy" target="_blank">{"Exact KUtrace view ↗"}</a></nav></header>
        <div class="view-toolbar"><div class="view-tabs" role="tablist">
          {for ["timeline", "legacy"].map(|view| { let active=*active_view==view; let active_view=active_view.clone(); let legacy_loaded=legacy_loaded.clone(); html!{<button class={classes!("view-tab", active.then_some("active"))} data-view={view} role="tab" aria-selected={active.to_string()} onclick={Callback::from(move |_| {active_view.set(view.to_owned()); if view=="legacy" {legacy_loaded.set(true)}})}>{if view=="timeline" {"Timeline"} else {"Exact KUtrace"}}</button>} })}
        </div><div class="search-tools"><label>{"Find "}<input id="trace-search" type="search" value={(*search).clone()} oninput={{let search=search.clone(); Callback::from(move |event: InputEvent| search.set(event.target_unchecked_into::<HtmlInputElement>().value()))}}/></label><button id="search-invert" aria-pressed={search_invert.to_string()} onclick={{let value=search_invert.clone(); Callback::from(move |_| value.set(!*value))}}>{"Not"}</button><span id="search-count" class="muted">{if search.is_empty() {String::new()} else {format!("{search_matches} matches")}}</span></div>
        <div class="range-controls"><button id="pan-left" onclick={navigate("pan-left")}>{"←"}</button><button id="zoom-out" onclick={navigate("zoom-out")}>{"−"}</button><button id="reset-range" onclick={navigate("reset")}>{"Fit"}</button><button id="zoom-in" onclick={navigate("zoom-in")}>{"+"}</button><button id="pan-right" onclick={navigate("pan-right")}>{"→"}</button></div></div>
        <main>
          <aside class="track-sidebar"><section><h2>{"Tracks"}</h2><label class="field-label">{"Group by"}<select id="track-mode" value={track_mode.value()} onchange={{let track_mode=track_mode.clone(); let highlighted=highlighted.clone(); Callback::from(move |event: Event| {let value=event.target_unchecked_into::<HtmlSelectElement>().value(); track_mode.set(match value.as_str(){"cpu"=>TrackMode::Cpu,"pid"=>TrackMode::Pid,_=>TrackMode::CpuPid}); highlighted.set(HashSet::new());})}}><option value="cpu_pid" selected={*track_mode==TrackMode::CpuPid}>{"KUtrace groups"}</option><option value="cpu" selected={*track_mode==TrackMode::Cpu}>{"CPU cores"}</option><option value="pid" selected={*track_mode==TrackMode::Pid}>{"Process / thread"}</option></select></label><div class="track-groups"><button class="track-group active" data-track-group="cpu">{"▾ CPUs"}</button><button class="track-group active" data-track-group="process">{"▾ Processes"}</button><button class="track-group active" data-track-group="rpc">{"▾ RPCs"}</button><button class="track-group active" data-track-group="resource">{"▾ Resources & queues"}</button></div></section>
          <section><h2>{"Display"}</h2><div class="display-toggles">{for [("marks","Mark"),("arcs","Arc"),("locks","Lock"),("frequency","Freq"),("ipc","IPC"),("samples","Samp"),("colorblind","CB")].map(|(key,label)| {let overlays_handle=overlays.clone(); let pressed=overlay_value(*overlays,key); html!{<button data-overlay={key} aria-pressed={pressed.to_string()} onclick={Callback::from(move |_| overlays_handle.set(toggle_overlay(*overlays_handle,key)))}>{label}</button>}})}</div></section>
          <section><h2>{"Composable filters"}</h2><div class="filter-form"><select id="filter-field" value={(*filter_field).clone()} onchange={{let value=filter_field.clone();Callback::from(move |event:Event|value.set(event.target_unchecked_into::<HtmlSelectElement>().value()))}}>{for ["category","cpu","pid","event","rpc","name"].map(|field|html!{<option value={field} selected={*filter_field==field}>{field}</option>})}</select><select id="filter-op" value={(*filter_op).clone()} onchange={{let value=filter_op.clone();Callback::from(move |event:Event|value.set(event.target_unchecked_into::<HtmlSelectElement>().value()))}}><option value="=" selected={*filter_op=="="}>{"is"}</option><option value="!=" selected={*filter_op=="!="}>{"is not"}</option><option value="contains" selected={*filter_op=="contains"}>{"contains"}</option><option value=">=" selected={*filter_op==">="}>{"≥"}</option><option value="<=" selected={*filter_op=="<="}>{"≤"}</option></select><input id="filter-value" value={(*filter_value).clone()} oninput={{let value=filter_value.clone();Callback::from(move |event:InputEvent|value.set(event.target_unchecked_into::<HtmlInputElement>().value()))}}/><button id="add-filter" onclick={add_filter}>{"Add filter"}</button></div><div id="filter-chips" class="chips">{for filters.iter().enumerate().map(|(index,filter)|{let filters=filters.clone();html!{<span class="chip">{format!("{} {} {}",filter.field,filter.op,filter.value)}<button data-remove={index.to_string()} onclick={Callback::from(move |_|{let mut next=(*filters).clone();next.remove(index);filters.set(next)})}>{"×"}</button></span>}})}</div></section>
          <section><h2>{"Saved SQL views"}</h2><div class="saved-view-form"><input id="view-name" maxlength="80" placeholder="Slow syscalls" value={(*view_name).clone()} oninput={{let view_name=view_name.clone();Callback::from(move |event:InputEvent|view_name.set(event.target_unchecked_into::<HtmlInputElement>().value()))}}/><button id="save-view" onclick={save_view}>{"Save current query"}</button></div><div id="saved-views" class={classes!("saved-views",saved_views.is_empty().then_some("muted"))}>{saved_view_rows}</div></section><section><button id="show-schema" onclick={show_schema}>{"Inspect SQL schema"}</button></section></aside>
          <div class="workspace">
          <section id="timeline-view" class={classes!("view-pane",(*active_view=="timeline").then_some("active"),(*dock_open).then_some("dock-open"))} hidden={*active_view!="timeline"}>
            <section class="timeline-card"><div class="section-head timeline-title"><div><h2>{"System timeline"}</h2><span id="renderer-label" class="mode-badge">{"Native KUtrace · Rust/WASM"}</span></div><div class="renderer-tabs"><button class="renderer-tab active" data-renderer="kutrace" aria-selected="true">{"Native KUtrace"}</button></div></div>
              <div id="kutrace-renderer" class="timeline-renderer modern-renderer active"><div class="modern-renderer-head"><span id="timeline-mode" class="mode-badge">{if timeline_source.ends_with("-partial") {"Density summary · partial"} else if *timeline_truncated {"Density summary"} else {"Exact vector events"}}</span><span id="range-label">{range_label(*range)}</span><div class="timeline-actions"><button id="zoom-selection" disabled={selection.is_none()} onclick={{let range=range.clone();let selection=selection.clone();Callback::from(move |_|if let Some(selected)=&*selection{range.set(selected.range)})}}>{"Zoom selection"}</button><button id="clear-selection" disabled={selection.is_none()} onclick={{let selection=selection.clone();Callback::from(move |_|selection.set(None))}}>{"Clear selection"}</button><span class="shortcut-help">{"drag select · Shift+click highlight · Ctrl+wheel · WASD"}</span></div></div>
              <Overview events={(*events).clone()} range={*range} full={metadata.full} colorblind={overlays.colorblind} on_range={set_range.clone()}/><div class="time-ruler"><span id="ruler-start">{format!("{:.6}s",range.start)}</span><span id="ruler-center">{format!("{:.6}s",(range.start+range.end)/2.0)}</span><span id="ruler-end">{format!("{:.6}s",range.end)}</span></div><div class="timeline-scroll"><div class="timeline-shell"><Timeline events={(*events).clone()} range={*range} full={metadata.full} mode={*track_mode} overlays={*overlays} search={(*search).clone()} search_invert={*search_invert} highlighted={(*highlighted).clone()} loading={*timeline_loading||!navigation_keys.is_empty()} truncated={*timeline_truncated} source={(*timeline_source).clone()} selection={(*selection).clone()} on_range={set_range} on_select={on_select} on_highlight={on_highlight}/></div></div>
              <div class="legend"><i class="agent"></i>{"agent "}<i class="syscall"></i>{"syscall "}<i class="kernel"></i>{"kernel "}<i class="user"></i>{"user "}<i class="scheduler"></i>{"scheduler "}<i class="special"></i>{"other "}<span id="perf-legend">{if metadata.flags&128!=0 {"◆ IPC"} else {""}}</span></div></div></section>
            <section class={classes!("analysis-dock",(!*dock_open).then_some("collapsed"))}><div class="dock-tabs">{for [("details","Details"),("flamegraph","Flamegraph"),("sql","SQL"),("agent","Agent reasoning")].map(|(name,label)|{let active=*active_dock==name;let active_dock=active_dock.clone();let dock_open=dock_open.clone();html!{<button class={classes!("dock-tab",active.then_some("active"))} data-dock={name} aria-selected={active.to_string()} onclick={Callback::from(move |_|{active_dock.set(name.to_owned());dock_open.set(true)})}>{label}</button>}})}<button id="toggle-dock" class="dock-toggle" aria-expanded={dock_open.to_string()} onclick={{let dock_open=dock_open.clone();Callback::from(move |_|dock_open.set(!*dock_open))}}>{if *dock_open{"⌄"}else{"⌃"}}</button></div><div class="dock-body">
              <section class={classes!("dock-panel",(*active_dock=="details").then_some("active"))} data-dock-panel="details"><div id="selection-summary" class="selection-summary">{selection.as_ref().map(|selected|if let Some(event)=&selected.event{format!("{} · {} · {:.6}s · {:.3} ms",event.name,event.category,event.start,event.duration*1000.0)}else{format!("Selected {}",range_label(selected.range))}).unwrap_or_else(||"Select an event or drag across tracks to inspect a region.".to_owned())}</div><div class="section-head"><h2>{"Events"}</h2><span id="event-count">{format!("{}{} rows",events.len(),if *timeline_truncated{"+"}else{""})}</span></div><div class="table-wrap"><table id="event-table"><thead><tr>{for ["ts","dur","cpu","pid","event","name","category","arg0","retval","ipc"].map(|name|html!{<th>{name}</th>})}</tr></thead><tbody>{for events.iter().take(500).map(|event|html!{<tr><td>{event.start}</td><td>{event.duration}</td><td>{event.cpu}</td><td>{event.pid}</td><td>{event.event}</td><td>{event.name.clone()}</td><td>{event.category.clone()}</td><td>{event.arg0}</td><td>{event.retval}</td><td>{event.ipc}</td></tr>})}</tbody></table></div></section>
              <section class={classes!("dock-panel",(*active_dock=="flamegraph").then_some("active"))} data-dock-panel="flamegraph"><div class="section-head"><div><h2>{"Callchain flamegraph"}</h2><span id="flame-range">{range_label(flame_range)}</span></div><div class="flame-tools"><select id="flame-weight" value={(*flame_weight).clone()} disabled={callchain_samples>0} onchange={{let value=flame_weight.clone();Callback::from(move |event:Event|value.set(event.target_unchecked_into::<HtmlSelectElement>().value()))}}><option value="duration" selected={*flame_weight=="duration"}>{"Duration"}</option><option value="count" selected={*flame_weight=="count"}>{"Event count"}</option></select><span id="flame-status" class="muted">{if callchain_samples>0{format!("{} · depth {}",*profile_status,callchain_depth)}else{format!("{} spans · {} · no sampled callchains",events.len(),*flame_weight)}}</span></div></div><p class="panel-note">{if callchain_samples>0{"Post-capture Blazesym callchains, weighted by samples."}else{"No optional sampled callchains in this trace; showing the event hierarchy fallback."}}</p><div id="flamegraph" class="flamegraph" style={format!("height:{}px",if callchain_samples>0{((callchain_depth+2)*29).max(120)}else{240})}><button class="flame-frame" data-flame-frame="true" data-flame-level="root" style="left:0%;top:2px;width:100%;background:#9fb7d7">{if callchain_samples>0{format!("root · {callchain_samples} samples")}else{format!("root · {} spans",events.len())}}</button>{for flame_frames}</div></section>
              <section class={classes!("dock-panel","sql-card",(*active_dock=="sql").then_some("active"))} data-dock-panel="sql"><div class="section-head"><div><h2>{"SQL notebook"}</h2><span>{"Read-only · result limit 10,000"}</span></div><button id="run-sql" onclick={run_sql}>{"Run query"}</button></div><textarea id="sql" value={(*sql).clone()} oninput={{let sql=sql.clone();Callback::from(move |event:InputEvent|sql.set(event.target_unchecked_into::<HtmlTextAreaElement>().value()))}}/><div id="query-status" class="muted">{if error.is_empty(){(*sql_status).clone()}else{(*error).clone()}}</div><div class="table-wrap"><DataTable id="query-table" response={(*sql_result).clone()}/></div></section>
              <section class={classes!("dock-panel","agent-panel",(*active_dock=="agent").then_some("active"))} data-dock-panel="agent"><div class="agent-grid"><section><h2>{"Agent call tree"}</h2><div id="agent-tree" class={classes!("tree",agent_nodes.is_empty().then_some("muted"))}>{if agent_nodes.is_empty(){html!{"No agent spans overlap the viewport."}}else{html!{<>{for agent_nodes}</>}}}</div></section><section><h2>{"RPC flows"}</h2><div id="rpc-flows" class={classes!("tree",related_rpcs.is_empty().then_some("muted"))}>{if related_rpcs.is_empty(){html!{"No overlapping RPC activity."}}else{html!{<>{for related_rpcs.iter().map(|event|html!{<div class="relation-row" data-related-rpc={event.rpc.to_string()}><b>{format!("RPC {}",event.rpc)}</b>{format!(" · {} · {:.3} ms",event.name,event.duration*1000.0)}</div>})}</>}}}</div><h2>{"Resources & queues"}</h2><div id="resource-activity" class={classes!("tree",related_resources.is_empty().then_some("muted"))}>{if related_resources.is_empty(){html!{"No overlapping resource activity."}}else{html!{<>{for related_resources.iter().map(|event|html!{<div class="relation-row" data-related-resource={event.arg0.to_string()}><b>{format!("RES {}",event.arg0)}</b>{format!(" · {} · {:.3} ms",event.name,event.duration*1000.0)}</div>})}</>}}}</div></section></div><section class="agent-context-card"><div class="context-actions"><span id="agent-context-title">{selected_agent.as_ref().map(|event|format!("{} · span {}",event.name,event.arg0)).unwrap_or_else(||"Select an agent span".to_owned())}</span><span id="agent-context-count" class="muted">{selected_agent.as_ref().map(|_|format!("{} annotations · {} RPC rows · {} resource rows",annotations.len(),related_rpcs.len(),related_resources.len())).unwrap_or_default()}</span><button id="open-agent-context" disabled={selected_agent.is_none()} onclick={open_agent_context}>{"Open SQL"}</button></div><div id="agent-annotations" class="annotation-list">{for annotations.iter().map(|event|html!{<span class="annotation-pill" data-annotation-kind={event.name.split('.').nth(1).unwrap_or("annotation").to_owned()}><b>{event.name.split('.').nth(1).unwrap_or("annotation")}</b>{format!(" · {} · value {}",event.name,event.arg0)}</span>})}</div><details open={selected_agent.is_some()}><summary>{"Exact bounded query"}</summary><pre id="agent-context-sql">{context_sql.clone().unwrap_or_default()}</pre></details><div class="table-wrap"><table id="agent-context-table"><thead><tr>{for ["ts","dur","cpu","pid","rpc","event","name"].map(|name|html!{<th>{name}</th>})}</tr></thead><tbody>{for events.iter().filter(|event|selected_agent.as_ref().is_some_and(|agent|event.start<agent.end&&event.end>agent.start)).take(100).map(|event|html!{<tr><td>{event.start}</td><td>{event.duration}</td><td>{event.cpu}</td><td>{event.pid}</td><td>{event.rpc}</td><td>{event.event}</td><td>{event.name.clone()}</td></tr>})}</tbody></table></div></section></section>
            </div></section>
          </section>
          <section id="legacy-view" class={classes!("view-pane","legacy-pane",(*active_view=="legacy").then_some("active"))} hidden={*active_view!="legacy"}><div class="legacy-head"><div><h2>{"Exact KUtrace viewer"}</h2><span>{"Original renderer"}</span></div><a href="/legacy" target="_blank">{"Open standalone ↗"}</a></div><iframe id="legacy-frame" title="Exact KUtrace viewer" data-src="/legacy" src={if *legacy_loaded {Some("/legacy")} else {None}}></iframe></section>
          </div>
        </main>
      </div>
    }
}
