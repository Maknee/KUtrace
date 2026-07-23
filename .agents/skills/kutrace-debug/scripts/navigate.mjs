#!/usr/bin/env node

import {fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';

const [url, ...actions] = process.argv.slice(2);
if (!url) {
  console.error('usage: navigate.mjs URL [ACTION...]');
  process.exit(2);
}

const repositoryRoot = fileURLToPath(new URL('../../../../', import.meta.url));
const require = createRequire(`${repositoryRoot}/ebpf/kutrace-ui/browser/package.json`);
const playwright = require('playwright');
const browser = await playwright.chromium.launch({headless: true});
const page = await browser.newPage({viewport: {width: 1440, height: 1000}});

async function ensureTrackVisible(track) {
  const label = page.locator(`.track-label[data-track="${track}"]`);
  if (await label.count() === 1) return label;

  const timeline = page.locator('#timeline');
  const scroll = page.locator('#timeline-scroll');
  await scroll.evaluate(element => element.scrollTop = 0);
  const rowCount = Number(await timeline.getAttribute('data-track-count'));
  for (let pageIndex = 0; pageIndex < rowCount + 1; pageIndex += 1) {
    if (await label.count() === 1) return label;
    const end = Number(await timeline.getAttribute('data-y-end'));
    if (end >= rowCount) break;
    const before = await timeline.getAttribute('data-y-start');
    await scroll.evaluate(element => element.scrollBy({top: element.clientHeight * .8, behavior: 'instant'}));
    await page.waitForFunction(
      previous => document.querySelector('#timeline')?.getAttribute('data-y-start') !== previous,
      before,
    );
  }
  throw new Error(`track not found in vertical viewport: ${track}`);
}

try {
  const response = await page.goto(url, {waitUntil: 'domcontentloaded'});
  if (!response?.ok()) {
    throw new Error(`navigation failed with HTTP ${response?.status()}`);
  }
  await page.locator('#timeline[data-ready="true"]').waitFor();

  for (const action of actions) {
    if (['zoom-in', 'zoom-out', 'pan-left', 'pan-right'].includes(action)) {
      await page.locator(`#${action}`).click();
    } else if (action === 'fit') {
      await page.locator('#reset-range').click();
    } else if (['y-zoom-in', 'y-zoom-out', 'y-fit'].includes(action)) {
      await page.locator(`#${action}`).click();
    } else if (action === 'y-pan-down' || action === 'y-pan-up') {
      const before = await page.locator('#timeline').getAttribute('data-y-scroll');
      const direction = action === 'y-pan-down' ? 1 : -1;
      await page.locator('#timeline-scroll').evaluate(
        (element, direction) => element.scrollBy({
          top: direction * element.clientHeight * .75,
          behavior: 'instant',
        }),
        direction,
      );
      await page.waitForFunction(
        previous => document.querySelector('#timeline')?.getAttribute('data-y-scroll') !== previous,
        before,
      );
    } else if (action.startsWith('dock:')) {
      const dock = action.slice('dock:'.length);
      await page.locator(`[data-dock="${dock}"]`).click();
    } else if (action.startsWith('agent:')) {
      const span = action.slice('agent:'.length);
      await page.locator('[data-dock="agent"]').click();
      await page.locator(`[data-agent-span="${span}"]`).click();
    } else if (action.startsWith('search:')) {
      await page.locator('#trace-search').fill(action.slice('search:'.length));
    } else if (action.startsWith('search-min:')) {
      await page.locator('#search-min').fill(action.slice('search-min:'.length));
    } else if (action.startsWith('search-max:')) {
      await page.locator('#search-max').fill(action.slice('search-max:'.length));
    } else if (action.startsWith('search-unit:')) {
      const requested = action.slice('search-unit:'.length);
      const target = {nsec: 'nsec', usec: 'µsec', msec: 'msec'}[requested];
      if (!target) throw new Error(`unknown search duration unit: ${requested}`);
      for (let attempt = 0; attempt < 3; attempt += 1) {
        if (await page.locator('#timeline').getAttribute('data-search-units') === target) break;
        await page.locator('#search-units').click();
      }
      if (await page.locator('#timeline').getAttribute('data-search-units') !== target) {
        throw new Error(`could not select search duration unit: ${requested}`);
      }
    } else if (action === 'search-not') {
      await page.locator('#search-invert').click();
    } else if (action.startsWith('view-save:')) {
      const slot = Number(action.slice('view-save:'.length));
      if (!Number.isInteger(slot) || slot < 1 || slot > 4) {
        throw new Error(`quick view save slot must be 1 through 4: ${slot}`);
      }
      await page.locator(`[data-view-slot="${slot}"]`).click({modifiers: ['Shift']});
    } else if (action.startsWith('view:')) {
      const slot = Number(action.slice('view:'.length));
      if (!Number.isInteger(slot) || slot < 1 || slot > 4) {
        throw new Error(`quick view restore slot must be 1 through 4: ${slot}`);
      }
      await page.locator(`[data-view-slot="${slot}"]`).click();
    } else if (action === 'view-back') {
      await page.locator('[data-view-slot="0"]').click();
    } else if (action.startsWith('display-shift:') || action.startsWith('display:')) {
      const shifted = action.startsWith('display-shift:');
      const display = action.slice((shifted ? 'display-shift:' : 'display:').length);
      if (![
        'marks', 'arcs', 'locks', 'frequency', 'ipc', 'samples',
        'annotate_user', 'annotate_all', 'colorblind',
      ].includes(display)) {
        throw new Error(`unknown display control: ${display}`);
      }
      await page.locator(`[data-overlay="${display}"]`).click({
        modifiers: shifted ? ['Shift'] : [],
      });
    } else if (action.startsWith('group:')) {
      const group = action.slice('group:'.length);
      if (!['cpu', 'pid', 'rpc', 'resource'].includes(group)) {
        throw new Error(`unknown track group: ${group}`);
      }
      const before = await page.locator('#timeline').getAttribute('data-track-group-states');
      await page.locator(`[data-track-group="${group}"]`).click();
      await page.waitForFunction(
        previous => document.querySelector('#timeline')?.getAttribute('data-track-group-states') !== previous,
        before,
      );
    } else if (action.startsWith('highlight:')) {
      const track = action.slice('highlight:'.length);
      const label = await ensureTrackVisible(track);
      await label.press('Enter');
    } else if (action.startsWith('track:')) {
      await ensureTrackVisible(action.slice('track:'.length));
    } else {
      throw new Error(`unknown action: ${action}`);
    }
    // Timeline fetches are deliberately delayed by 70 ms so held navigation
    // stays smooth. Let that debounce begin before asserting the settled state;
    // otherwise an agent can observe the previous ready=true frame.
    await page.waitForTimeout(90);
    await page.locator('#timeline[data-ready="true"]').waitFor();
  }

  const timeline = page.locator('#timeline');
  const result = {
    title: (await page.locator('#trace-title').textContent())?.trim() ?? '',
    range: (await page.locator('#range-label').textContent())?.trim() ?? '',
    source: await timeline.getAttribute('data-source'),
    detail: await timeline.getAttribute('data-detail'),
    visibleTracks: (await timeline.getAttribute('data-visible-tracks') ?? '')
      .split(',')
      .filter(Boolean),
    trackGroups: (await timeline.getAttribute('data-track-groups') ?? '')
      .split(',')
      .filter(Boolean),
    trackGroupStates: Object.fromEntries(
      (await timeline.getAttribute('data-track-group-states') ?? '')
        .split(',')
        .filter(Boolean)
        .map(value => value.split(':', 2)),
    ),
    highlightedTracks: (await timeline.getAttribute('data-highlighted-tracks') ?? '')
      .split(',')
      .filter(Boolean),
    renderedEvents: Number(await timeline.getAttribute('data-rendered-events')),
    rowCount: Number(await timeline.getAttribute('data-track-count')),
    rowHeight: Number(await timeline.getAttribute('data-row-height')),
    verticalScroll: Number(await timeline.getAttribute('data-y-scroll')),
    visibleRowRange: [
      Number(await timeline.getAttribute('data-y-start')),
      Number(await timeline.getAttribute('data-y-end')),
    ],
    trackCatalogTruncated: await timeline.getAttribute('data-track-catalog-truncated') === 'true',
    displayStates: Object.fromEntries(
      await page.locator('[data-overlay]').evaluateAll(buttons => buttons.map(button => [
        button.getAttribute('data-overlay'),
        Number(button.getAttribute('data-state')),
      ])),
    ),
    annotatedEvents: await page.locator('.trace-event[data-annotated="true"]').count(),
    timelineGlyphs: {
      rpcMessages: Number(await timeline.getAttribute('data-rpc-messages')),
      networkPackets: Number(await timeline.getAttribute('data-network-packets')),
      wakeupArcs: await page.locator('[data-overlay-glyph="arc"]').count(),
    },
    selection: (await page.locator('#selection-summary').textContent())?.trim() ?? '',
    agentContext: (await page.locator('#agent-context-title').textContent())?.trim() ?? '',
    searchMatches: (await page.locator('#search-count').textContent())?.trim() ?? '',
    search: {
      text: await page.locator('#trace-search').inputValue(),
      minimum: await page.locator('#search-min').inputValue(),
      maximum: await page.locator('#search-max').inputValue(),
      units: await timeline.getAttribute('data-search-units'),
      mode: await timeline.getAttribute('data-search-mode'),
      invert: await timeline.getAttribute('data-search-invert') === 'true',
      matches: Number(await timeline.getAttribute('data-search-count')),
    },
    viewSlots: Object.fromEntries(
      await page.locator('[data-view-slot]').evaluateAll(buttons => buttons.map(button => [
        button.getAttribute('data-view-slot'),
        button.getAttribute('data-saved') === 'true',
      ])),
    ),
  };
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
} finally {
  await browser.close();
}
