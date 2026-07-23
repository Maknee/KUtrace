# Original KUtrace UI parity checklist

The authoritative behavior reference is
[`linux/setup/postproc/show_cpu.html`](../linux/setup/postproc/show_cpu.html).
The modern Rust/WASM view is complete only when each applicable row below is
implemented and exercised against representative trace data. `/legacy` is a
comparison surface, not a substitute for reimplementation.

| Original behavior | Modern status | Verification or remaining work |
|---|---|---|
| White canvas, blue controls/labels, black execution rails | implemented | Chromium `workspace.png` visual baseline |
| One continuous horizontal time domain | implemented | WASD, wheel, Alt-drag, selection, overview browser tests |
| Vector detail remains sharp while zooming | implemented | Geometry changes are asserted after held-key and wheel navigation |
| CPU, PID, RPC, and resource group ordering | implemented | Fixture asserts aligned labels and lane order |
| Independently cycle the four groups | implemented | Full/hidden without highlights and full/highlight-only/hidden with highlights have Chromium/Firefox regressions |
| Auto-prune rows without positive spans in the visible range | implemented | Track derivation and visible-core tests |
| Shift-toggle line/event highlights with dim surrounding context | implemented | Shift-click and keyboard line-label toggles propagate event emphasis across CPU/PID/RPC/resource copies |
| Three-state group display: hidden, highlight-only, full | implemented | Original skip-highlight-only rule without selected rows is preserved in exact and density modes |
| Vertical-axis pan and zoom for very large row sets | implemented | Catalog-backed virtual viewport, native Y scrolling, label-wheel zoom, explicit controls, and 128-core/128-PID browser gate |
| Original CPU/PID/RPC/resource Y sorting rules | implemented | CPU/PID/resource IDs are numeric; RPC IDs retain first-occurrence order as in `show_cpu.html` |
| Mark, arc, lock, frequency, IPC, sample, and color-blind controls | implemented | Original click and Shift-click multi-state cycles have Rust and Chromium/Firefox regressions |
| User/all annotation modes and callout placement | implemented | Mutually exclusive user/all modes retain bounded on-canvas labels; legacy event `-4` renders the original blue-on-white rounded bubble and triangular notch, with independently draggable bubble placement and a tip that snaps across visible CPU/PID/RPC rows. A real-legacy comparison plus Chromium/Firefox drag and visual regressions cover both handles |
| Text search and inverse search | implemented | Browser search/inversion regression |
| Duration min/max search and `CPUI`/`CPUU`/`CPUK`/`RPC`/`PID`/`RES` syntax | implemented | Inclusive nsec/µsec/msec bounds and all six documented selectors share Rust and Chromium/Firefox matching tests |
| Search respects visible X and Y ranges | implemented | Toolbar and timeline counts are restricted to the current time and virtualized row viewport |
| Quick saved viewport slots and restore | implemented | Back plus four Shift-save/click-restore slots preserve the complete investigative view and have Chromium/Firefox and agent-navigation regressions |
| Tiny-span deferral/density refinement | implemented | Exact-to-density budget and mipmap tests |
| Specialized RPC packet/message and wakeup glyphs | implemented | A deterministic wire fixture exercises one shared network band, red RX/cyan TX slopes, packet ticks, packetized dashes, RPC IDs, legacy 10ns size reconstruction, independent arc toggling, color-blind colors, and agent-visible glyph counts in Chromium/Firefox |
| Idle, wait, user stripes, lock rails, frequency, and IPC glyph grammar | implemented | A deterministic fixture is rendered through both the original gzip/hex viewer pipeline and the Rust/WASM view; Chromium owns the differential baseline while Chromium/Firefox assert solid/dashed idle, two/four-line rails, Morse wait colors, stacked held/try braces, frequency bands, IPC needles, display cycles, and agent-visible glyph counts |
| Exact event inspection | implemented differently | Human details dock and bounded SQL expose exact rows |
| Sampled-stack flamegraph | modern extension | Normalized symbolized callchain browser fixture |
| Agent reasoning/RPC/resource context | modern extension | Agent tree, exact SQL, annotations, and browser navigation helper |

## Acceptance method

For each remaining original behavior:

1. Add a minimal deterministic version-3 trace fixture containing the required
   event family and edge cases.
2. Capture the original renderer at fixed viewport/range/state.
3. Reimplement the behavior in Rust/Yew/SVG without embedding the original.
4. Add Chromium visual evidence and Chromium/Firefox interaction assertions.
5. Exercise the same state through
   `.agents/skills/kutrace-debug/scripts/navigate.mjs` when it affects viewport
   navigation.
