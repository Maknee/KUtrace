use std::{
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
};

use gloo_timers::callback::Timeout;
use wasm_bindgen::JsCast;
use web_sys::{Element, KeyboardEvent, MouseEvent, PointerEvent, WheelEvent};
use yew::prelude::*;

use crate::model::{
    Overlays, Range, SearchSpec, TraceEvent, TrackGroupMode, TrackGroups, TrackMode,
};

const VIEW_WIDTH: f64 = 1_400.0;
const LABEL_WIDTH: f64 = 116.0;
pub const DEFAULT_ROW_HEIGHT: f64 = 52.0;
pub const MIN_ROW_HEIGHT: f64 = 18.0;
pub const MAX_ROW_HEIGHT: f64 = 96.0;
const ROW_OVERSCAN: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub range: Range,
    pub event: Option<TraceEvent>,
}

#[derive(Properties, PartialEq)]
pub struct TimelineProps {
    pub events: Rc<Vec<TraceEvent>>,
    pub track_catalog: Rc<Vec<String>>,
    pub catalog_truncated: bool,
    pub range: Range,
    pub full: Range,
    pub mode: TrackMode,
    pub groups: TrackGroups,
    pub overlays: Overlays,
    pub search: SearchSpec,
    pub highlighted: HashSet<String>,
    pub loading: bool,
    pub truncated: bool,
    pub source: String,
    pub row_height: f64,
    pub vertical_scroll: f64,
    pub viewport_height: f64,
    pub selection: Option<Selection>,
    pub on_range: Callback<Range>,
    pub on_select: Callback<Option<Selection>>,
    pub on_highlight: Callback<String>,
    pub on_row_zoom: Callback<(f64, f64)>,
}

fn event_visible(event: &TraceEvent, overlays: Overlays) -> bool {
    match event.category.as_str() {
        "mark" | "annotation" => {
            overlays.marks > 0
                && match event.event {
                    0x20a | 0x20c => overlays.marks & 2 != 0,
                    0x20b | 0x20d => overlays.marks & 1 != 0,
                    _ => true,
                }
        }
        "rpc" | "wakeup" => overlays.arcs > 0,
        "lock" => overlays.locks > 0,
        "sample" => {
            (overlays.samples == 1 && event.pid == 0) || (overlays.samples == 2 && event.pid != 0)
        }
        _ if event.event == 521 || event.event == 540 => overlays.frequency > 0,
        _ => true,
    }
}

fn event_overlaps(event: &TraceEvent, range: Range) -> bool {
    if event.duration <= 0.0 {
        event.start >= range.start && event.start <= range.end
    } else {
        event.start < range.end && event.end > range.start
    }
}

fn event_colors(event: i64, colorblind: bool) -> (&'static str, &'static str) {
    const LIGHT: [&str; 17] = [
        "#f7b6d2", "#a8e6cf", "#ffd3a5", "#b5d8ff", "#d5b3ff", "#ffe58a", "#b8f2e6", "#ffcab1",
        "#c7ceea", "#c9f0c1", "#f6c1c7", "#a0d8ef", "#e2c2ff", "#f9e2ae", "#bde0fe", "#cdeac0",
        "#ffc8dd",
    ];
    const DARK: [&str; 15] = [
        "#b00060", "#008060", "#d06000", "#0050b0", "#7030a0", "#a07800", "#007c78", "#b04020",
        "#3f51a3", "#348c31", "#a52a3a", "#166088", "#663399", "#8a5a00", "#1d5d9b",
    ];
    if colorblind {
        return ("#b9dcf2", "#0072b2");
    }
    let value = event.unsigned_abs() as usize;
    (
        LIGHT[(value * 4) % LIGHT.len()],
        DARK[(value * 7 + 6) % DARK.len()],
    )
}

fn category_fill(category: &str, colorblind: bool) -> &'static str {
    match (category, colorblind) {
        ("agent" | "annotation" | "mark", true) => "#cc79a7",
        ("syscall", true) => "#009e73",
        ("kernel", true) => "#e69f00",
        ("scheduler", true) => "#d55e00",
        ("rpc", true) => "#0072b2",
        ("resource", true) => "#f0e442",
        ("user", true) => "#56b4e9",
        ("agent" | "annotation" | "mark", false) => "#d783ff",
        ("syscall", false) => "#53d6a5",
        ("kernel", false) => "#ffb454",
        ("scheduler", false) => "#f07178",
        ("rpc", false) => "#4f71c6",
        ("resource", false) => "#c94e86",
        ("user", false) => "#6ea8fe",
        _ => "#7d899c",
    }
}

fn track_group(track: &str) -> &str {
    track.split_once(':').map_or("", |(group, _)| group)
}

fn group_has_highlight(group: &str, highlighted: &HashSet<String>) -> bool {
    highlighted.iter().any(|track| track_group(track) == group)
}

fn track_visible(track: &str, groups: TrackGroups, highlighted: &HashSet<String>) -> bool {
    let group = track_group(track);
    match groups.mode(group) {
        TrackGroupMode::Hidden => false,
        TrackGroupMode::Full => true,
        TrackGroupMode::Highlighted => {
            !group_has_highlight(group, highlighted) || highlighted.contains(track)
        }
    }
}

fn event_is_highlighted(event: &TraceEvent, highlighted: &HashSet<String>) -> bool {
    if highlighted.is_empty() {
        return true;
    }
    event
        .render_track
        .as_ref()
        .is_some_and(|track| highlighted.contains(track))
        || (event.cpu >= 0 && highlighted.contains(&format!("cpu:{}", event.cpu)))
        || (event.pid > 0 && highlighted.contains(&format!("pid:{}", event.pid)))
        || (event.rpc > 0 && highlighted.contains(&format!("rpc:{}", event.rpc)))
        || (event.category == "resource"
            && event.arg0 >= 0
            && highlighted.contains(&format!("resource:{}", event.arg0)))
}

fn track_keys(
    events: &[TraceEvent],
    catalog: &[String],
    groups: TrackGroups,
    highlighted: &HashSet<String>,
    range: Range,
) -> Vec<String> {
    if !catalog.is_empty() {
        return catalog
            .iter()
            .filter(|track| track_visible(track, groups, highlighted))
            .cloned()
            .collect();
    }
    let mut cpus = BTreeSet::new();
    let mut pids = BTreeSet::new();
    let mut rpcs = HashMap::<i64, f64>::new();
    let mut resources = BTreeSet::new();
    for event in events {
        if event.duration <= 0.0 || event.start >= range.end || event.end <= range.start {
            continue;
        }
        if let Some(track) = &event.render_track {
            if let Some(cpu) = track
                .strip_prefix("cpu:")
                .and_then(|value| value.parse().ok())
            {
                cpus.insert(cpu);
            }
            if let Some(pid) = track
                .strip_prefix("pid:")
                .and_then(|value| value.parse().ok())
            {
                pids.insert(pid);
            }
            if let Some(rpc) = track
                .strip_prefix("rpc:")
                .and_then(|value| value.parse().ok())
            {
                rpcs.entry(rpc)
                    .and_modify(|first| *first = first.min(event.start))
                    .or_insert(event.start);
            }
            if let Some(resource) = track
                .strip_prefix("resource:")
                .and_then(|value| value.parse().ok())
            {
                resources.insert(resource);
            }
        } else if event.pid > 0 && event.cpu >= 0 {
            cpus.insert(event.cpu);
            pids.insert(event.pid);
        }
        if event.rpc > 0 {
            rpcs.entry(event.rpc)
                .and_modify(|first| *first = first.min(event.start))
                .or_insert(event.start);
        }
        if event.category == "resource" && event.arg0 >= 0 {
            resources.insert(event.arg0);
        }
    }
    let mut tracks = Vec::new();
    if groups.enabled("cpu") {
        tracks.extend(cpus.into_iter().map(|cpu| format!("cpu:{cpu}")));
    }
    if groups.enabled("pid") {
        tracks.extend(pids.into_iter().map(|pid| format!("pid:{pid}")));
    }
    if groups.enabled("rpc") {
        let mut ordered_rpcs = rpcs.into_iter().collect::<Vec<_>>();
        ordered_rpcs.sort_by(|(left_id, left_ts), (right_id, right_ts)| {
            left_ts
                .total_cmp(right_ts)
                .then_with(|| left_id.cmp(right_id))
        });
        tracks.extend(
            ordered_rpcs
                .into_iter()
                .map(|(rpc, _)| format!("rpc:{rpc}")),
        );
    }
    if groups.enabled("resource") {
        tracks.extend(
            resources
                .into_iter()
                .map(|resource| format!("resource:{resource}")),
        );
    }
    tracks.retain(|track| track_visible(track, groups, highlighted));
    tracks
}

fn track_label(track: &str) -> String {
    if let Some(cpu) = track.strip_prefix("cpu:") {
        format!("CPU {cpu}")
    } else if let Some(pid) = track.strip_prefix("pid:") {
        format!("PID {pid}")
    } else if let Some(rpc) = track.strip_prefix("rpc:") {
        format!("RPC {rpc}")
    } else if let Some(resource) = track.strip_prefix("resource:") {
        format!("RES {resource}")
    } else {
        track.to_owned()
    }
}

fn x_at(time: f64, range: Range) -> f64 {
    LABEL_WIDTH + (time - range.start) * (VIEW_WIDTH - LABEL_WIDTH) / range.span()
}

fn pointer_time(event: &PointerEvent, range: Range, timeline: &NodeRef) -> Option<f64> {
    let element = timeline.cast::<Element>()?;
    let rect = element.get_bounding_client_rect();
    if !rect.width().is_finite() || rect.width() <= f64::EPSILON {
        return None;
    }
    let logical_x = (event.client_x() as f64 - rect.left()) * VIEW_WIDTH / rect.width();
    let fraction = ((logical_x - LABEL_WIDTH) / (VIEW_WIDTH - LABEL_WIDTH)).clamp(0.0, 1.0);
    Some(range.start + range.span() * fraction)
}

fn logical_x(client_x: i32, timeline: &NodeRef) -> Option<f64> {
    let element = timeline.cast::<Element>()?;
    let rect = element.get_bounding_client_rect();
    if !rect.width().is_finite() || rect.width() <= f64::EPSILON {
        return None;
    }
    Some((client_x as f64 - rect.left()) * VIEW_WIDTH / rect.width())
}

#[function_component(Timeline)]
pub fn timeline(props: &TimelineProps) -> Html {
    // Pointer motion is kept outside component state so a selection drag does
    // not reconcile thousands of SVG event nodes on every mouse event.
    let drag_start = use_mut_ref(|| None::<f64>);
    let drag_pan = use_mut_ref(|| false);
    let drag_origin = use_mut_ref(|| None::<Range>);
    let suppress_click = use_mut_ref(|| false);
    let drag_overlay = use_node_ref();
    let timeline_node = use_node_ref();
    let tracks = track_keys(
        &props.events,
        &props.track_catalog,
        props.groups,
        &props.highlighted,
        props.range,
    );
    let row_height = props.row_height.clamp(MIN_ROW_HEIGHT, MAX_ROW_HEIGHT);
    let viewport_height = props.viewport_height.max(240.0);
    let first_visible_row = ((props.vertical_scroll / row_height).floor().max(0.0) as usize)
        .saturating_sub(ROW_OVERSCAN)
        .min(tracks.len());
    let last_visible_row =
        (((props.vertical_scroll + viewport_height) / row_height).ceil() as usize + ROW_OVERSCAN)
            .min(tracks.len());
    let row_index: HashMap<&str, usize> = tracks
        .iter()
        .enumerate()
        .map(|(index, track)| (track.as_str(), index))
        .collect();
    let height = (tracks.len().max(1) as f64 * row_height + 20.0).max(viewport_height);
    let track_groups = props.groups.names();
    let visible_tracks = tracks[first_visible_row..last_visible_row].join(",");
    let highlighted_tracks = {
        let mut values = props.highlighted.iter().cloned().collect::<Vec<_>>();
        values.sort();
        values.join(",")
    };
    // Associate each visible event with its enabled KUtrace rows in one pass. The old
    // track × event nested scan became quadratic on many-core traces and also
    // rendered every event in the five-viewport prefetch margin at an edge.
    let mut rendered_events = Vec::<(String, usize, &TraceEvent)>::new();
    for event in props
        .events
        .iter()
        .filter(|event| event_overlaps(event, props.range) && event_visible(event, props.overlays))
    {
        let mut targets = Vec::with_capacity(4);
        if let Some(track) = &event.render_track {
            targets.push(track.clone());
        } else if event.pid > 0 && event.cpu >= 0 {
            if props.groups.enabled("cpu") {
                targets.push(format!("cpu:{}", event.cpu));
            }
            if props.groups.enabled("pid") {
                targets.push(format!("pid:{}", event.pid));
            }
        }
        if props.groups.enabled("rpc") && event.rpc > 0 {
            targets.push(format!("rpc:{}", event.rpc));
        }
        if props.groups.enabled("resource") && event.category == "resource" && event.arg0 >= 0 {
            targets.push(format!("resource:{}", event.arg0));
        }
        targets.sort();
        targets.dedup();
        for track in targets {
            if let Some(index) = row_index.get(track.as_str()).copied() {
                if (first_visible_row..last_visible_row).contains(&index) {
                    rendered_events.push((track, index, event));
                }
            }
        }
    }
    let rendered_event_count = rendered_events.len();
    let search_count = rendered_events
        .iter()
        .filter_map(|(_, _, event)| props.search.matches(event).then_some(event.id))
        .collect::<HashSet<_>>()
        .len();
    let mut annotation_tracks = HashMap::<i64, String>::new();
    if props.overlays.annotations > 0 {
        let mut seen_user_events = HashSet::new();
        for (track, _, event) in &rendered_events {
            if annotation_tracks.len() >= 64
                || event.name.is_empty()
                || !event_is_highlighted(event, &props.highlighted)
            {
                continue;
            }
            if props.overlays.annotations == 1
                && (!matches!(event.category.as_str(), "user" | "agent")
                    || !seen_user_events.insert(event.event))
            {
                continue;
            }
            annotation_tracks
                .entry(event.id)
                .or_insert_with(|| track.clone());
        }
    }

    let onpointerdown = {
        let drag_start = drag_start.clone();
        let drag_pan = drag_pan.clone();
        let drag_origin = drag_origin.clone();
        let suppress_click = suppress_click.clone();
        let drag_overlay = drag_overlay.clone();
        let timeline_node = timeline_node.clone();
        let range = props.range;
        Callback::from(move |event: PointerEvent| {
            if event.button() == 0 {
                if let Some(time) = pointer_time(&event, range, &timeline_node) {
                    event.prevent_default();
                    if let Some(element) = timeline_node.cast::<Element>() {
                        element.set_pointer_capture(event.pointer_id()).ok();
                    }
                    *drag_start.borrow_mut() = Some(time);
                    *drag_pan.borrow_mut() = event.alt_key();
                    *drag_origin.borrow_mut() = event.alt_key().then_some(range);
                    *suppress_click.borrow_mut() = false;
                    if let Some(overlay) = drag_overlay.cast::<Element>() {
                        overlay.set_attribute("visibility", "hidden").ok();
                    }
                }
            }
        })
    };
    let onpointermove = {
        let drag_start = drag_start.clone();
        let drag_pan = drag_pan.clone();
        let drag_origin = drag_origin.clone();
        let suppress_click = suppress_click.clone();
        let drag_overlay = drag_overlay.clone();
        let timeline_node = timeline_node.clone();
        let range = props.range;
        let full = props.full;
        let on_range = props.on_range.clone();
        Callback::from(move |event: PointerEvent| {
            if let Some(start) = *drag_start.borrow() {
                let basis = (*drag_origin.borrow()).unwrap_or(range);
                if let Some(time) = pointer_time(&event, basis, &timeline_node) {
                    if *drag_pan.borrow() {
                        let delta = start - time;
                        on_range.emit(
                            Range {
                                start: basis.start + delta,
                                end: basis.end + delta,
                            }
                            .bounded(full),
                        );
                    } else if let Some(overlay) = drag_overlay.cast::<Element>() {
                        if (start - time).abs() > basis.span() / (VIEW_WIDTH - LABEL_WIDTH) {
                            *suppress_click.borrow_mut() = true;
                        }
                        let x = x_at(start.min(time), basis);
                        let width = (x_at(start.max(time), basis) - x).max(1.0);
                        overlay.set_attribute("x", &x.to_string()).ok();
                        overlay.set_attribute("width", &width.to_string()).ok();
                        overlay.set_attribute("visibility", "visible").ok();
                    }
                }
            }
        })
    };
    let onpointerup = {
        let drag_start = drag_start.clone();
        let drag_pan = drag_pan.clone();
        let drag_origin = drag_origin.clone();
        let suppress_click = suppress_click.clone();
        let drag_overlay = drag_overlay.clone();
        let timeline_node = timeline_node.clone();
        let on_select = props.on_select.clone();
        let range = props.range;
        Callback::from(move |event: PointerEvent| {
            let basis = (*drag_origin.borrow()).unwrap_or(range);
            if !*drag_pan.borrow() {
                if let (Some(a), Some(b)) = (
                    *drag_start.borrow(),
                    pointer_time(&event, basis, &timeline_node),
                ) {
                    if (a - b).abs() > f64::EPSILON {
                        on_select.emit(Some(Selection {
                            range: Range {
                                start: a.min(b),
                                end: a.max(b),
                            },
                            event: None,
                        }));
                    }
                }
            }
            if let Some(element) = timeline_node.cast::<Element>() {
                element.release_pointer_capture(event.pointer_id()).ok();
            }
            if let Some(overlay) = drag_overlay.cast::<Element>() {
                overlay.set_attribute("visibility", "hidden").ok();
            }
            *drag_start.borrow_mut() = None;
            *drag_pan.borrow_mut() = false;
            *drag_origin.borrow_mut() = None;
            if *suppress_click.borrow() {
                let suppress_click = suppress_click.clone();
                Timeout::new(0, move || *suppress_click.borrow_mut() = false).forget();
            }
        })
    };
    let onpointercancel = {
        let drag_start = drag_start.clone();
        let drag_pan = drag_pan.clone();
        let drag_origin = drag_origin.clone();
        let suppress_click = suppress_click.clone();
        let drag_overlay = drag_overlay.clone();
        Callback::from(move |_event: PointerEvent| {
            if let Some(overlay) = drag_overlay.cast::<Element>() {
                overlay.set_attribute("visibility", "hidden").ok();
            }
            *drag_start.borrow_mut() = None;
            *drag_pan.borrow_mut() = false;
            *drag_origin.borrow_mut() = None;
            *suppress_click.borrow_mut() = false;
        })
    };
    let onwheel = {
        let range = props.range;
        let full = props.full;
        let vertical_scroll = props.vertical_scroll;
        let on_range = props.on_range.clone();
        let on_row_zoom = props.on_row_zoom.clone();
        let timeline_node = timeline_node.clone();
        Callback::from(move |event: WheelEvent| {
            if logical_x(event.client_x(), &timeline_node).is_some_and(|x| x < LABEL_WIDTH) {
                event.prevent_default();
                event.stop_propagation();
                let next_height = (row_height * (-event.delta_y() * 0.002).exp())
                    .clamp(MIN_ROW_HEIGHT, MAX_ROW_HEIGHT);
                let desired_scroll = timeline_node
                    .cast::<Element>()
                    .map(|element| {
                        let rect = element.get_bounding_client_rect();
                        let absolute_y = event.client_y() as f64 - rect.top();
                        let local_y = absolute_y - vertical_scroll;
                        (absolute_y / row_height * next_height - local_y).max(0.0)
                    })
                    .unwrap_or(vertical_scroll * next_height / row_height);
                on_row_zoom.emit((next_height, desired_scroll));
                return;
            }
            if event.ctrl_key() || event.meta_key() {
                event.prevent_default();
                let factor = (event.delta_y() * 0.002).exp();
                let center = (range.start + range.end) / 2.0;
                let width = range.span() * factor;
                on_range.emit(
                    Range {
                        start: center - width / 2.0,
                        end: center + width / 2.0,
                    }
                    .bounded(full),
                );
            }
        })
    };
    let selection_overlay = props.selection.as_ref().map(|selection| {
        let x = x_at(selection.range.start, props.range);
        let width = (x_at(selection.range.end, props.range) - x).max(1.0);
        html! {<rect id="time-selection" x={x.to_string()} y="0" width={width.to_string()} height={height.to_string()} class="time-selection-vector"/>}
    });
    html! {
      <div class="timeline-vector-shell">
        <svg id="timeline"
          ref={timeline_node}
          class="timeline-vector"
          viewBox={format!("0 0 {VIEW_WIDTH} {height}")}
          preserveAspectRatio="none"
          style={format!("height:{height}px")}
          tabindex="0"
          aria-label="KUtrace vector timeline; drag to select, Shift click a span to highlight its track, Ctrl wheel to zoom"
          data-ready={(!props.loading).to_string()}
          data-source={props.source.clone()}
          data-detail={(!props.truncated).to_string()}
          data-renderer="kutrace"
          data-preview={props.loading.to_string()}
          data-preview-mode="vector"
          data-track-mode={props.mode.value()}
          data-track-groups={track_groups}
          data-track-group-states={props.groups.states()}
          data-track-count={tracks.len().to_string()}
          data-track-catalog-truncated={props.catalog_truncated.to_string()}
          data-visible-tracks={visible_tracks}
          data-row-height={format!("{row_height:.3}")}
          data-y-scroll={format!("{:.3}",props.vertical_scroll)}
          data-y-start={first_visible_row.to_string()}
          data-y-end={last_visible_row.to_string()}
          data-highlighted-tracks={highlighted_tracks}
          data-search-count={search_count.to_string()}
          data-search-mode={props.search.mode()}
          data-search-min={props.search.minimum.clone()}
          data-search-max={props.search.maximum.clone()}
          data-search-units={props.search.units.label()}
          data-search-invert={props.search.invert.to_string()}
          data-rendered-events={rendered_event_count.to_string()}
          {onpointerdown} {onpointermove} {onpointerup} {onpointercancel} {onwheel}>
          <rect x="0" y="0" width={VIEW_WIDTH.to_string()} height={height.to_string()} fill="#fff"/>
          { for (0..=10).map(|tick| {
              let x = LABEL_WIDTH + (VIEW_WIDTH - LABEL_WIDTH) * tick as f64 / 10.0;
              let time = props.range.start + props.range.span() * tick as f64 / 10.0;
              html! {<g class="time-grid"><line x1={x.to_string()} y1="0" x2={x.to_string()} y2={height.to_string()}/><text x={x.to_string()} y="12">{format!("{:.3}", time * 1000.0)}</text></g>}
          })}
          { for tracks.iter().enumerate().skip(first_visible_row).take(last_visible_row-first_visible_row).map(|(index, track)| {
              let row_top = index as f64 * row_height;
              let center = row_top + row_height / 2.0;
              let emphasized = props.highlighted.is_empty() || props.highlighted.contains(track);
              let selected = props.highlighted.contains(track);
              let track_copy = track.clone();
              let on_highlight = props.on_highlight.clone();
              let onclick = Callback::from(move |event: MouseEvent| {
                  event.stop_propagation();
                  if event.shift_key() {
                      on_highlight.emit(track_copy.clone());
                  }
              });
              let track_copy = track.clone();
              let on_highlight = props.on_highlight.clone();
              let onkeydown = Callback::from(move |event: KeyboardEvent| {
                  if matches!(event.key().as_str(),"Enter"|" ") {
                      event.prevent_default();
                      event.stop_propagation();
                      on_highlight.emit(track_copy.clone());
                  }
              });
              let onpointerdown = Callback::from(|event: PointerEvent| {
                  event.stop_propagation();
              });
              html! {<g class={classes!("track-row", (!emphasized).then_some("dimmed"))}>
                <rect x="0" y={row_top.to_string()} width={VIEW_WIDTH.to_string()} height={row_height.to_string()} class="track-background"/>
                <text x="8" y={(center+4.0).to_string()}
                  class={classes!("track-label",selected.then_some("highlighted"))}
                  data-track={track.clone()}
                  tabindex="0" role="button" aria-pressed={selected.to_string()}
                  aria-label={format!("Highlight {}",track_label(track))}
                  {onclick} {onkeydown} {onpointerdown}>{track_label(track)}</text>
                <line x1={LABEL_WIDTH.to_string()} y1={center.to_string()} x2={VIEW_WIDTH.to_string()} y2={center.to_string()} class="track-center"/>
              </g>}
          })}
          { for rendered_events.into_iter().map(|(track, index, event)| {
              let center = index as f64 * row_height + row_height / 2.0;
                  let x = x_at(event.start.max(props.range.start), props.range);
                  let end = event.end.max(event.start + props.range.span() / (VIEW_WIDTH - LABEL_WIDTH));
                  let width = (x_at(end.min(props.range.end), props.range) - x).max(0.8);
                  let matched = props.search.matches(event);
                  let emphasized = event_is_highlighted(event, &props.highlighted);
                  let (light, dark) = event_colors(event.event, props.overlays.colorblind);
                  let fill = if event.duration <= 0.0 { category_fill(&event.category, props.overlays.colorblind) } else { light };
                  let opacity = if emphasized && (!props.search.active() || matched) { 0.96 } else { 0.16 };
                  let event_copy = event.clone();
                  let track_copy = track.clone();
                  let on_select = props.on_select.clone();
                  let on_highlight = props.on_highlight.clone();
                  let suppress_click = suppress_click.clone();
                  let onclick = Callback::from(move |mouse: MouseEvent| {
                      mouse.stop_propagation();
                      if *suppress_click.borrow() {
                          mouse.prevent_default();
                          return;
                      }
                      if mouse.shift_key() {
                          on_highlight.emit(track_copy.clone());
                      } else {
                          on_select.emit(Some(Selection {
                              range: Range { start: event_copy.start, end: event_copy.end.max(event_copy.start + f64::EPSILON) },
                              event: Some(event_copy.clone()),
                          }));
                      }
                  });
                  let is_mark = matches!(event.category.as_str(), "mark" | "annotation");
                  let is_sample = event.category == "sample";
                  let is_lock = event.category == "lock";
                  let is_frequency = event.event == 521 || event.event == 540;
                  let is_idle = event.event == 65_536;
                  let is_wait = event.event & 0x0f_ffe0 == 768;
                  let wake_target = if event.category == "wakeup" && track.starts_with("cpu:") {
                      row_index.get(format!("pid:{}", event.arg0).as_str()).copied().map(|target| target as f64 * row_height + row_height / 2.0)
                  } else { None };
                  let event_height = (row_height - 8.0).clamp(8.0, 24.0);
                  let annotation_height = (row_height - 4.0).clamp(10.0, 40.0);
                  let h = if is_mark || is_sample { annotation_height } else { event_height };
                  let y = center - h / 2.0;
                  let annotated = row_height >= 32.0
                      && annotation_tracks.get(&event.id).is_some_and(|annotated_track| annotated_track == &track);
                  let ipc_visible = event.ipc != 0
                      && ((matches!(event.category.as_str(),"user"|"agent") && props.overlays.ipc&1 != 0)
                          || (!matches!(event.category.as_str(),"user"|"agent") && props.overlays.ipc&2 != 0));
                  html! {<g class="trace-event" opacity={opacity.to_string()} data-annotated={annotated.to_string()} {onclick}>
                    <title>{format!("{} · {} · {} · {:.9}s · {:.2}us", if event.name.is_empty() {"(unnamed)"} else {&event.name}, event.category, track_label(&track), event.start, event.duration.max(0.0)*1e6)}</title>
                    if is_mark {
                      <path data-overlay-glyph="mark" d={format!("M {x} {y} l -5 10 h 10 z")} fill={dark}/>
                    } else if is_sample {
                      <line data-overlay-glyph="sample" x1={x.to_string()} y1={y.to_string()} x2={x.to_string()} y2={(y+h).to_string()} stroke={dark} stroke-width="1.5"/>
                    } else if let Some(target) = wake_target {
                      <path data-overlay-glyph="arc" d={format!("M {x} {center} Q {} {} {} {target}",x+18.0,(center+target)/2.0-18.0,x+30.0)} fill="none" stroke={if props.overlays.arcs==1{"#0055aa"}else{"#0080ff"}} stroke-width={if props.overlays.arcs==1{"3"}else{"2"}} stroke-dasharray={if props.overlays.arcs==1{"4 4"}else{"3 3"}} marker-end="url(#arrowhead)"/>
                    } else if is_lock {
                      <line data-overlay-glyph="lock" x1={x.to_string()} y1={(center-15.0).to_string()} x2={(x+width).to_string()} y2={(center-15.0).to_string()} stroke={dark} stroke-width={if props.overlays.locks==1{"4"}else{"2"}} stroke-dasharray={if event.event&1==0{"none"}else{"5 3"}}/>
                    } else if is_frequency {
                      <rect data-overlay-glyph="frequency" x={x.to_string()} y={(center-18.0).to_string()} width={width.to_string()} height={if props.overlays.frequency==1{"11"}else{"7"}} fill="#50b45a" opacity={if props.overlays.frequency==1{".8"}else{".45"}}/>
                      if width > 34.0 {<text x={(x+3.0).to_string()} y={(center-12.0).to_string()} class="event-label">{format!("{}MHz",event.arg0)}</text>}
                    } else if is_idle || is_wait {
                      <line x1={x.to_string()} y1={center.to_string()} x2={(x+width).to_string()} y2={center.to_string()} stroke="#111" stroke-width={if is_idle{"2"}else{"1.5"}} stroke-dasharray={if is_wait{"5 3"}else{"none"}}/>
                    } else {
                      <rect x={x.to_string()} y={y.to_string()} width={width.to_string()} height={h.to_string()} rx="1" fill={fill} stroke={dark} stroke-width="1"/>
                      if matches!(event.category.as_str(),"user"|"agent") && width > 3.0 {
                        <line x1={(x+1.0).to_string()} y1={(center-5.0).to_string()} x2={(x+width-1.0).to_string()} y2={(center-5.0).to_string()} stroke={dark} stroke-width="1"/>
                        <line x1={(x+1.0).to_string()} y1={(center+5.0).to_string()} x2={(x+width-1.0).to_string()} y2={(center+5.0).to_string()} stroke={dark} stroke-width="1"/>
                      }
                      if width > 34.0 {
                        <text x={(x+3.0).to_string()} y={(center+3.5).to_string()} class="event-label">{event.name.clone()}</text>
                      }
                      if ipc_visible {
                        <line data-overlay-glyph="ipc" x1={x.to_string()} y1={y.to_string()} x2={(x+width).to_string()} y2={y.to_string()} stroke="#fff" stroke-width="2"/>
                      }
                    }
                    if annotated {
                      <line class="canvas-annotation" x1={x.to_string()} y1={(center-h/2.0).to_string()} x2={x.to_string()} y2={(index as f64*row_height+2.0).to_string()}/>
                      <text class="canvas-annotation-text" x={(x+3.0).to_string()} y={(index as f64*row_height+11.0).to_string()} transform={format!("rotate(-24 {} {})",x+3.0,index as f64*row_height+11.0)}>{format!("{} · {:.2}µs",event.name,event.duration.max(0.0)*1e6)}</text>
                    }
                  </g>}
          })}
          <defs><marker id="arrowhead" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto"><path d="M0,0 L7,3.5 L0,7 z" fill="#be0000"/></marker></defs>
          {selection_overlay}
          <rect ref={drag_overlay} x="0" y="0" width="1" height={height.to_string()} visibility="hidden" class="time-selection-vector dragging"/>
        </svg>
      </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct OverviewProps {
    pub events: Rc<Vec<TraceEvent>>,
    pub range: Range,
    pub full: Range,
    pub colorblind: bool,
    pub on_range: Callback<Range>,
}

#[function_component(Overview)]
pub fn overview(props: &OverviewProps) -> Html {
    let histogram = use_memo((props.events.clone(), props.full), |(events, full)| {
        let bins = 200usize;
        let mut counts = vec![0usize; bins];
        for event in events.iter() {
            let bin = (((event.start - full.start) / full.span()) * bins as f64)
                .floor()
                .clamp(0.0, (bins - 1) as f64) as usize;
            counts[bin] += 1;
        }
        let max = counts.iter().copied().max().unwrap_or(1).max(1) as f64;
        (counts, max)
    });
    let (counts, max) = &*histogram;
    let left = 100.0 * (props.range.start - props.full.start) / props.full.span();
    let width = 100.0 * props.range.span() / props.full.span();
    let full = props.full;
    let current = props.range;
    let on_range = props.on_range.clone();
    let onclick = Callback::from(move |event: MouseEvent| {
        let Some(element) = event
            .current_target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let rect = element.get_bounding_client_rect();
        let fraction = ((event.client_x() as f64 - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let center = full.start + full.span() * fraction;
        on_range.emit(
            Range {
                start: center - current.span() / 2.0,
                end: center + current.span() / 2.0,
            }
            .bounded(full),
        );
    });
    html! {
      <div class="overview-shell" aria-label="Full trace overview">
        <svg id="overview" viewBox="0 0 1000 32" preserveAspectRatio="none" {onclick}>
          <rect x="0" y="0" width="1000" height="32" fill="#ffffff"/>
          {for counts.iter().enumerate().map(|(index, count)| {
              let h = (*count as f64 / *max * 28.0).max(if *count > 0 {1.0} else {0.0});
              html!{<rect x={(index as f64*5.0).to_string()} y={(32.0-h).to_string()} width="5.2" height={h.to_string()} fill={if props.colorblind {"#56b4e9"} else {"#5f91df"}}/>}
          })}
        </svg>
        <div id="overview-viewport" style={format!("left:{}%;width:{}%", left.max(0.0), width.min(100.0))}></div>
      </div>
    }
}
