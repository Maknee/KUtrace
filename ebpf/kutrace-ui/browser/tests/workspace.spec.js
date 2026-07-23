import {expect, test} from '@playwright/test';
import {readFile} from 'node:fs/promises';

async function openDock(page, name) {
  await page.locator(`[data-dock="${name}"]`).click();
  await expect(page.locator(`[data-dock="${name}"]`)).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator(`[data-dock-panel="${name}"]`)).toBeVisible();
}

async function waitForTimeline(page) {
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready', 'true');
}

test.beforeEach(async ({page}) => {
  const response = await page.goto('/');
  expect(response.headers()['cache-control']).toBe('no-store');
  await expect(page.locator('#trace-title')).toContainText('Agent reasoning fixture');
  await waitForTimeline(page);
});

test('serves a Yew/WASM vector workspace without application JavaScript', async ({page}) => {
  await expect(page.locator('#timeline')).toHaveJSProperty('tagName', 'svg');
  await expect(page.locator('#timeline')).toHaveAttribute('data-renderer', 'kutrace');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source', 'events');
  await expect(page.locator('#timeline')).toHaveAttribute('data-detail', 'true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-track-mode', 'cpu_pid');
  await expect(page.locator('#timeline')).toHaveAttribute('data-track-groups', 'cpu,pid,rpc,resource');
  await expect(page.locator('#timeline')).toHaveAttribute(
    'data-track-group-states',
    'cpu:full,pid:full,rpc:full,resource:full',
  );
  await expect(page.locator('#timeline')).toHaveAttribute('data-visible-tracks', /cpu:.+,pid:.+,rpc:77,resource:12,resource:900/);
  await expect(page.locator('#track-mode')).toHaveValue('cpu_pid');
  await expect(page.locator('#renderer-label')).toContainText('Rust/WASM');
  await expect(page.locator('#timeline-mode')).toContainText('Exact vector events');
  expect(await page.locator('#timeline canvas').count()).toBe(0);
  await expect(page.locator('#timeline .track-label')).toContainText([
    'CPU 0', 'CPU 1', 'PID 100', 'PID 101', 'RPC 77', 'RES 12', 'RES 900'
  ]);

  const root = await page.request.get('/');
  expect(await root.text()).toContain('/wasm/kutrace-ui-web.js');
  expect((await page.request.get('/app.js')).status()).toBe(404);
  const wasm = await page.request.get('/wasm/kutrace-ui-web_bg.wasm');
  expect(wasm.headers()['content-type']).toBe('application/wasm');
  expect(wasm.headers()['cache-control']).toBe('no-store');
});

test('uses the original KUtrace light visual grammar and a deterministic baseline', async ({page, browserName}) => {
  test.skip(browserName !== 'chromium', 'Chromium owns the visual baseline');
  await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  await expect(page.locator('#timeline .track-label').first()).toHaveCSS('fill', 'rgb(0, 0, 204)');
  await expect(page.locator('#timeline .track-center').first()).toHaveCSS('stroke', 'rgb(17, 17, 17)');
  await expect(page).toHaveScreenshot('workspace.png', {
    animations: 'disabled',
    caret: 'hide',
    fullPage: true,
  });
});

test('independently expands and collapses original KUtrace track groups', async ({page}) => {
  const timeline = page.locator('#timeline');
  for (const group of ['cpu', 'pid', 'rpc', 'resource']) {
    await expect(page.locator(`[data-track-group="${group}"]`)).toHaveAttribute('aria-expanded', 'true');
  }

  const pidGroup = page.locator('[data-track-group="pid"]');
  await pidGroup.focus();
  await page.keyboard.press('Enter');
  await waitForTimeline(page);
  await expect(pidGroup).toHaveAttribute('aria-expanded', 'false');
  await expect(pidGroup).toHaveAttribute('data-group-state', 'hidden');
  await expect(timeline).toHaveAttribute('data-track-groups', 'cpu,rpc,resource');
  await expect(timeline).toHaveAttribute('data-visible-tracks', /cpu:.+,rpc:77,resource:12/);
  expect((await timeline.getAttribute('data-visible-tracks')).split(',').some(track => track.startsWith('pid:'))).toBe(false);
  await expect(timeline.locator('.track-label', {hasText: 'PID 100'})).toHaveCount(0);

  await page.locator('[data-track-group="rpc"]').click();
  await waitForTimeline(page);
  await expect(timeline).toHaveAttribute('data-track-groups', 'cpu,resource');
  await expect(timeline.locator('.track-label', {hasText: 'RPC 77'})).toHaveCount(0);

  await page.locator('[data-track-group="cpu"]').click();
  await page.locator('[data-track-group="resource"]').click();
  await waitForTimeline(page);
  await page.locator('#trace-search').fill('agent');
  await expect(page.locator('#search-count')).toHaveText('0 matches');
  await page.locator('#trace-search').fill('');

  await page.locator('#track-mode').selectOption('pid');
  await waitForTimeline(page);
  await expect(timeline).toHaveAttribute('data-track-groups', 'pid');
  await expect(page.locator('[data-track-group="pid"]')).toHaveAttribute('aria-expanded', 'true');
  await expect(page.locator('[data-track-group="cpu"]')).toHaveAttribute('aria-expanded', 'false');
  await expect(timeline).toHaveAttribute('data-visible-tracks', /^pid:/);

  await page.locator('#track-mode').selectOption('cpu_pid');
  await waitForTimeline(page);
  await expect(timeline).toHaveAttribute('data-track-groups', 'cpu,pid,rpc,resource');
});

test('matches original three-state groups and line-label highlighting', async ({page}) => {
  const timeline = page.locator('#timeline');
  const cpuGroup = page.locator('[data-track-group="cpu"]');
  const cpu0 = timeline.locator('.track-label[data-track="cpu:0"]');

  await cpu0.click({modifiers: ['Shift']});
  await waitForTimeline(page);
  await expect(cpu0).toHaveAttribute('aria-pressed', 'true');
  await expect(timeline).toHaveAttribute('data-highlighted-tracks', 'cpu:0');

  await cpuGroup.click();
  await waitForTimeline(page);
  await expect(cpuGroup).toHaveAttribute('data-group-state', 'highlighted');
  await expect(timeline).toHaveAttribute(
    'data-track-group-states',
    'cpu:highlighted,pid:full,rpc:full,resource:full',
  );
  const highlightedOnlyTracks = (await timeline.getAttribute('data-visible-tracks')).split(',');
  expect(highlightedOnlyTracks.filter(track => track.startsWith('cpu:'))).toEqual(['cpu:0']);

  await cpuGroup.click();
  await waitForTimeline(page);
  await expect(cpuGroup).toHaveAttribute('data-group-state', 'hidden');
  expect((await timeline.getAttribute('data-visible-tracks')).split(',').some(track => track.startsWith('cpu:'))).toBe(false);

  await cpuGroup.click();
  await waitForTimeline(page);
  await expect(cpuGroup).toHaveAttribute('data-group-state', 'full');
  await expect(timeline).toHaveAttribute('data-visible-tracks', /cpu:0,cpu:1/);

  await timeline.locator('.track-label[data-track="cpu:0"]').press('Enter');
  await waitForTimeline(page);
  await expect(timeline).toHaveAttribute('data-highlighted-tracks', '');
});

test('keeps vector geometry sharp through smooth WASD, wheel, and Alt-drag navigation', async ({page}) => {
  const timeline = page.locator('#timeline');
  const initial = await page.locator('#range-label').textContent();
  await page.keyboard.down('KeyW');
  await page.waitForTimeout(80);
  const first = await page.locator('#range-label').textContent();
  await page.waitForTimeout(100);
  const second = await page.locator('#range-label').textContent();
  expect(first).not.toBe(initial);
  expect(second).not.toBe(first);
  await expect(timeline).toHaveAttribute('data-preview-mode', 'vector');
  await page.keyboard.up('KeyW');
  await waitForTimeline(page);

  const beforeHeldPan = await timeline.locator('.trace-event').count();
  await page.keyboard.down('KeyD');
  await page.waitForTimeout(180);
  expect(await timeline.locator('.trace-event').count()).toBeGreaterThan(0);
  expect(await timeline.locator('.trace-event').count()).toBeGreaterThanOrEqual(Math.min(1, beforeHeldPan));
  await expect(timeline).toHaveAttribute('data-preview-mode', 'vector');
  await page.keyboard.up('KeyD');
  await waitForTimeline(page);

  const beforeWheel = await page.locator('#range-label').textContent();
  const box = await timeline.boundingBox();
  await page.keyboard.down('Control');
  await page.mouse.move(box.x + box.width * .6, box.y + 70);
  await page.mouse.wheel(0, -400);
  await page.keyboard.up('Control');
  await expect(page.locator('#range-label')).not.toHaveText(beforeWheel);

  const beforePan = await page.locator('#range-label').textContent();
  await page.keyboard.down('Alt');
  await page.mouse.move(box.x + box.width * .7, box.y + 70);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * .5, box.y + 70, {steps: 5});
  await page.mouse.up();
  await page.keyboard.up('Alt');
  await expect(page.locator('#range-label')).not.toHaveText(beforePan);
  await expect(page.locator('#time-selection')).toHaveCount(0);

  const before = await timeline.locator('.trace-event rect').evaluateAll(nodes => nodes.map(node => [node.getAttribute('x'), node.getAttribute('width')]));
  await page.locator('#zoom-in').click();
  await expect.poll(() => timeline.locator('.trace-event rect').evaluateAll(nodes => nodes.map(node => [node.getAttribute('x'), node.getAttribute('width')]))).not.toEqual(before);
  await expect(timeline).toHaveCSS('shape-rendering', /auto|geometricprecision/i);
});

test('maps mouse selection through the SVG viewport without lag or letterbox drift', async ({page}) => {
  await page.setViewportSize({width: 2000, height: 1000});
  const timeline = page.locator('#timeline');
  await waitForTimeline(page);
  const geometry = await timeline.evaluate(element => {
    const matrix = element.getScreenCTM();
    const rect = element.getBoundingClientRect();
    return {
      startX: matrix.e + matrix.a * 400,
      endX: matrix.e + matrix.a * 1000,
      y: rect.y + rect.height / 2,
    };
  });
  const label = await page.locator('#range-label').textContent();
  const [rangeStart, rangeEnd] = [...label.matchAll(/([0-9.]+)s/g)].map(match => Number(match[1]));
  const expected = logicalX => rangeStart + (logicalX - 116) / (1400 - 116) * (rangeEnd - rangeStart);

  await page.mouse.move(geometry.startX, geometry.y);
  await page.mouse.down();
  await page.mouse.move(geometry.endX, geometry.y, {steps: 8});
  const drag = timeline.locator('.time-selection-vector.dragging');
  await expect(drag).toHaveAttribute('visibility', 'visible');
  const dragX = Number(await drag.getAttribute('x'));
  const dragWidth = Number(await drag.getAttribute('width'));
  expect(Math.abs(dragX - 400)).toBeLessThan(1);
  expect(Math.abs(dragWidth - 600)).toBeLessThan(1);
  await page.mouse.up();

  const summary = await page.locator('#selection-summary').textContent();
  expect(summary).toContain('Selected');
  const [selectedStart, selectedEnd] = [...summary.matchAll(/([0-9.]+)s/g)].map(match => Number(match[1]));
  expect(Math.abs(selectedStart - expected(dragX))).toBeLessThan(1e-6);
  expect(Math.abs(selectedEnd - expected(dragX + dragWidth))).toBeLessThan(1e-6);
});

test('supports persistent selection, Shift track highlighting, search, filters, and Escape', async ({page}) => {
  const timeline = page.locator('#timeline');
  const event = timeline.locator('.trace-event').first();
  await event.dispatchEvent('click', {shiftKey: true, bubbles: true});
  await expect(timeline).toHaveAttribute('data-highlighted-tracks', /.+/);
  await event.dispatchEvent('click', {shiftKey: true, bubbles: true});
  await expect(timeline).toHaveAttribute('data-highlighted-tracks', '');

  const box = await timeline.boundingBox();
  await page.mouse.move(box.x + box.width * .3, box.y + box.height * .5);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * .7, box.y + box.height * .5, {steps: 5});
  await page.mouse.up();
  await expect(page.locator('#time-selection')).toBeVisible();
  await expect(page.locator('#zoom-selection')).toBeEnabled();
  await expect(page.locator('#selection-summary')).toContainText('Selected');
  await page.locator('#clear-selection').click();
  await expect(page.locator('#time-selection')).toHaveCount(0);

  await page.locator('#trace-search').fill('agent.tool');
  await expect(page.locator('#search-count')).toContainText('2 matches');
  await expect(timeline).toHaveAttribute('data-search-count', '2');
  await page.locator('#search-invert').click();
  await expect(page.locator('#search-invert')).toHaveAttribute('aria-pressed', 'true');

  const colorblind = page.locator('[data-overlay="colorblind"]');
  await colorblind.click();
  await expect(colorblind).toHaveAttribute('aria-pressed', 'true');

  await page.locator('#filter-field').selectOption('category');
  await page.locator('#filter-value').fill('agent');
  await page.locator('#add-filter').click();
  await waitForTimeline(page);
  await expect(page.locator('#filter-chips')).toContainText('category = agent');
  await expect(page.locator('#event-count')).toContainText('4 rows');
  await page.locator('#filter-value').focus();
  await page.keyboard.press('Escape');
  await expect(timeline).toBeFocused();
});

test('matches original duration bounds and special search selectors', async ({page}) => {
  const timeline = page.locator('#timeline');
  await expect(page.locator('#search-units')).toHaveText('µsec:');

  await page.locator('#trace-search').fill('agent.tool');
  await page.locator('#search-min').fill('3000');
  await page.locator('#search-max').fill('5000');
  await expect(page.locator('#search-count')).toContainText('1 matches');
  await expect(timeline).toHaveAttribute('data-search-count', '1');
  await expect(timeline).toHaveAttribute('data-search-mode', 'text');
  await expect(timeline).toHaveAttribute('data-search-min', '3000');
  await expect(timeline).toHaveAttribute('data-search-max', '5000');
  await expect(timeline).toHaveAttribute('data-search-units', 'µsec');

  await page.locator('#search-units').click();
  await page.locator('#search-min').fill('3');
  await page.locator('#search-max').fill('5');
  await expect(page.locator('#search-units')).toHaveText('msec:');
  await expect(page.locator('#search-count')).toContainText('1 matches');

  await page.locator('#search-min').fill('');
  await page.locator('#search-max').fill('');
  await page.locator('#trace-search').fill('CPUK');
  await expect(timeline).toHaveAttribute('data-search-mode', 'cpuk');
  await expect(timeline).toHaveAttribute('data-search-count', '2');

  await page.locator('#trace-search').fill('RES');
  await expect(timeline).toHaveAttribute('data-search-mode', 'resource');
  await expect(timeline).toHaveAttribute('data-search-count', '3');

  await page.locator('#trace-search').fill('CPUI');
  await expect(timeline).toHaveAttribute('data-search-mode', 'cpui');
  await expect(timeline).toHaveAttribute('data-search-count', '0');
  await page.locator('#search-invert').click();
  await expect(timeline).toHaveAttribute('data-search-invert', 'true');
  await expect(page.locator('#search-invert')).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(async () => Number(await timeline.getAttribute('data-search-count'))).toBeGreaterThan(0);

  await page.locator('#search-invert').click();
  for (const [selector, mode, matches] of [
    ['CPUU', 'cpuu', '5'],
    ['RPC', 'rpc', '6'],
    ['PID', 'pid', '18'],
  ]) {
    await page.locator('#trace-search').fill(selector);
    await expect(timeline).toHaveAttribute('data-search-mode', mode);
    await expect(timeline).toHaveAttribute('data-search-count', matches);
  }
});

test('saves and restores complete quick views with a Back slot', async ({page}) => {
  const slot = number => page.locator(`[data-view-slot="${number}"]`);
  await expect(slot(0)).toHaveAttribute('data-saved', 'false');
  for (const number of [1, 2, 3, 4]) {
    await expect(slot(number)).toHaveAttribute('data-saved', 'false');
  }

  await page.locator('#zoom-in').click();
  await page.locator('#y-zoom-in').click();
  await page.locator('#trace-search').fill('CPUK');
  await page.locator('[data-track-group="rpc"]').click();
  const savedRange = await page.locator('#range-label').textContent();
  const savedRowHeight = await page.locator('#timeline').getAttribute('data-row-height');
  await slot(1).click({modifiers: ['Shift']});
  await expect(slot(1)).toHaveAttribute('data-saved', 'true');

  await page.locator('#pan-right').click();
  await page.locator('#y-zoom-in').click();
  await page.locator('#trace-search').fill('RES');
  await page.locator('[data-track-group="rpc"]').click();
  const replacedRange = await page.locator('#range-label').textContent();
  const replacedRowHeight = await page.locator('#timeline').getAttribute('data-row-height');
  expect(replacedRange).not.toBe(savedRange);
  expect(replacedRowHeight).not.toBe(savedRowHeight);

  await slot(1).click();
  await expect(page.locator('#range-label')).toHaveText(savedRange);
  await expect(page.locator('#timeline')).toHaveAttribute('data-row-height', savedRowHeight);
  await expect(page.locator('#trace-search')).toHaveValue('CPUK');
  await expect(page.locator('[data-track-group="rpc"]')).toHaveAttribute('data-group-state', 'hidden');
  await expect(slot(0)).toHaveAttribute('data-saved', 'true');

  await slot(0).click();
  await expect(page.locator('#range-label')).toHaveText(replacedRange);
  await expect(page.locator('#timeline')).toHaveAttribute('data-row-height', replacedRowHeight);
  await expect(page.locator('#trace-search')).toHaveValue('RES');
  await expect(page.locator('[data-track-group="rpc"]')).toHaveAttribute('data-group-state', 'full');
});

test('matches original multi-state display cycles and annotation modes', async ({page}) => {
  const display = name => page.locator(`[data-overlay="${name}"]`);
  for (const [name, state] of Object.entries({
    marks: '3',
    arcs: '2',
    locks: '2',
    frequency: '2',
    ipc: '0',
    samples: '0',
    annotate_user: '0',
    annotate_all: '0',
    colorblind: '0',
  })) {
    await expect(display(name)).toHaveAttribute('data-state', state);
  }

  await expect(page.locator('[data-overlay-glyph="mark"]').first()).toBeVisible();
  for (const state of ['2', '1', '0', '3']) {
    await display('marks').click();
    await expect(display('marks')).toHaveAttribute('data-state', state);
    if (state === '0') {
      await expect(page.locator('[data-overlay-glyph="mark"]')).toHaveCount(0);
    }
  }
  await expect(page.locator('[data-overlay-glyph="mark"]').first()).toBeVisible();

  const arcs = page.locator('[data-overlay-glyph="arc"]');
  await expect(arcs.first()).toHaveAttribute('stroke-width', '2');
  for (const state of ['1', '0', '2']) {
    await display('arcs').click();
    await expect(display('arcs')).toHaveAttribute('data-state', state);
    if (state === '1') {
      await expect(arcs.first()).toHaveAttribute('stroke-width', '3');
    } else if (state === '0') {
      await expect(arcs).toHaveCount(0);
    }
  }
  await expect(arcs.first()).toHaveAttribute('stroke-width', '2');
  for (const state of ['3', '2', '1', '0']) {
    await display('ipc').click();
    await expect(display('ipc')).toHaveAttribute('data-state', state);
  }

  await display('samples').click();
  await expect(display('samples')).toHaveAttribute('data-state', '2');
  await display('samples').click();
  await expect(display('samples')).toHaveAttribute('data-state', '0');
  for (const state of ['2', '1', '0']) {
    await display('samples').click({modifiers: ['Shift']});
    await expect(display('samples')).toHaveAttribute('data-state', state);
  }

  await display('annotate_user').click();
  await expect(display('annotate_user')).toHaveAttribute('aria-pressed', 'true');
  await expect(display('annotate_all')).toHaveAttribute('aria-pressed', 'false');
  const userAnnotations = await page.locator('.trace-event[data-annotated="true"]').count();
  expect(userAnnotations).toBeGreaterThan(0);

  await display('annotate_all').click();
  await expect(display('annotate_user')).toHaveAttribute('aria-pressed', 'false');
  await expect(display('annotate_all')).toHaveAttribute('aria-pressed', 'true');
  const allAnnotations = await page.locator('.trace-event[data-annotated="true"]').count();
  expect(allAnnotations).toBeGreaterThanOrEqual(userAnnotations);

  await page.locator('#trace-search').fill('agent.tool');
  await expect(display('annotate_user')).toHaveAttribute('aria-pressed', 'false');
  await expect(display('annotate_all')).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('.trace-event[data-annotated="true"]')).toHaveCount(0);
});

test('draws original directional RPC messages, packets, and independent wakeup arcs', async ({page, browserName}) => {
  await page.goto('http://127.0.0.1:39130/');
  await expect(page.locator('#trace-title')).toContainText('RPC wire grammar fixture');
  await waitForTimeline(page);

  const timeline = page.locator('#timeline');
  await expect(timeline).toHaveAttribute('data-rpc-messages', '4');
  await expect(timeline).toHaveAttribute('data-network-packets', '2');

  const receiveMessages = page.locator('[data-overlay-glyph="rpc-message"][data-direction="rx"]');
  const transmitMessages = page.locator('[data-overlay-glyph="rpc-message"][data-direction="tx"]');
  const receivePackets = page.locator('[data-overlay-glyph="network-packet"][data-direction="rx"]');
  const transmitPackets = page.locator('[data-overlay-glyph="network-packet"][data-direction="tx"]');
  await expect(receiveMessages).toHaveCount(2);
  await expect(transmitMessages).toHaveCount(2);
  await expect(receivePackets).toHaveCount(1);
  await expect(transmitPackets).toHaveCount(1);
  await expect(receiveMessages.first().locator('[data-wire-segment]')).toHaveAttribute('stroke', '#800000');
  await expect(transmitMessages.first().locator('[data-wire-segment]')).toHaveAttribute('stroke', '#008080');
  await expect(receiveMessages.first().locator('[data-wire-segment]')).not.toHaveAttribute('stroke-dasharray', 'none');
  await expect(page.locator('.rpc-message-label').first()).toContainText('17');

  const arcs = page.locator('[data-overlay-glyph="arc"]');
  await expect(arcs.first()).toBeVisible();
  await page.locator('[data-overlay="arcs"]').click();
  await page.locator('[data-overlay="arcs"]').click();
  await expect(page.locator('[data-overlay="arcs"]')).toHaveAttribute('data-state', '0');
  await expect(arcs).toHaveCount(0);
  await expect(receiveMessages.first()).toBeVisible();
  await expect(transmitPackets.first()).toBeVisible();

  await page.locator('[data-overlay="arcs"]').click();
  await page.locator('[data-overlay="colorblind"]').click();
  await expect(receiveMessages.first().locator('[data-wire-segment]')).toHaveAttribute('stroke', '#d55e00');
  await expect(transmitMessages.first().locator('[data-wire-segment]')).toHaveAttribute('stroke', '#0072b2');

  if (browserName === 'chromium') {
    await page.locator('[data-overlay="colorblind"]').click();
    await expect(page.locator('.timeline-card')).toHaveScreenshot('rpc-wire-glyphs.png', {
      animations: 'disabled',
      caret: 'hide',
    });
  }
});

test('uses a real bounded density summary and preserves CPU/PID rows', async ({page}) => {
  let sawMipmap = false;
  let sawPidSummary = false;
  let sawLongEventMerge = false;
  await page.route('**/api/query', async route => {
    const request = route.request();
    if (request.method() === 'POST') {
      const sql = request.postDataJSON().sql;
      if (sql.startsWith('SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc FROM events') && sql.includes('ORDER BY ts,dur DESC LIMIT 10001')) {
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({columns: [], rows: [], truncated: true, elapsed_ms: .1, sql}),
        });
        return;
      }
      sawMipmap ||= sql.includes('FROM timeline_mipmap');
      sawPidSummary ||= sql.includes('WITH RECURSIVE scoped') && sql.includes('pid,event,name,category,ipc,ts,ts_end') && sql.includes('FROM events');
      sawLongEventMerge ||= sql.includes('(dur=0 OR dur>0.256)');
    }
    await route.continue();
  });
  await page.reload();
  await waitForTimeline(page);
  const timeline = page.locator('#timeline');
  await expect(timeline).toHaveAttribute('data-source', 'summary');
  await expect(timeline).toHaveAttribute('data-detail', 'false');
  await expect(timeline).toHaveAttribute('data-visible-tracks', /cpu:.+,pid:/);
  expect(sawMipmap).toBe(true);
  expect(sawPidSummary).toBe(true);
  expect(sawLongEventMerge).toBe(true);
});

test('keeps collapsed KUtrace groups out of density queries', async ({page}) => {
  const queriedTracks = new Set();
  await page.route('**/api/query', async route => {
    const request = route.request();
    if (request.method() === 'POST') {
      const sql = request.postDataJSON().sql;
      if (sql.startsWith('SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc FROM events') && sql.includes('ORDER BY ts,dur DESC LIMIT 10001')) {
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({columns: [], rows: [], truncated: true, elapsed_ms: .1, sql}),
        });
        return;
      }
      for (const track of ['cpu', 'pid', 'rpc', 'resource']) {
        if (sql.includes(`'${track}:' || track`)) queriedTracks.add(track);
      }
    }
    await route.continue();
  });
  await page.reload();
  await waitForTimeline(page);
  queriedTracks.clear();

  await page.locator('[data-track-group="pid"]').click();
  await expect.poll(() => [...queriedTracks].sort()).toEqual(['cpu', 'resource', 'rpc']);
  await waitForTimeline(page);
  await expect(page.locator('#timeline')).toHaveAttribute('data-track-groups', 'cpu,rpc,resource');
});

test('switches to density before exact SVG glyphs exceed the interaction budget', async ({page}) => {
  await page.route('**/api/query', async route => {
    const request = route.request();
    if (request.method() === 'POST') {
      const sql = request.postDataJSON().sql;
      if (sql.startsWith('SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc FROM events') && sql.includes('ORDER BY ts,dur DESC LIMIT 10001')) {
        const rows = Array.from({length: 501}, (_, index) => [index + 1, 1 + index / 100000, .00001, 1 + index / 100000 + .00001, 0, 100, 0, 65537, 'dense', 'user', 0, 0, 0]);
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({columns: [], rows, truncated: false, elapsed_ms: .1, sql}),
        });
        return;
      }
    }
    await route.continue();
  });
  await page.reload();
  await waitForTimeline(page);
  const timeline = page.locator('#timeline');
  await expect(timeline).toHaveAttribute('data-source', 'summary');
  expect(Number(await timeline.getAttribute('data-rendered-events'))).toBeLessThan(1000);
});

test('virtualizes every catalogued CPU and PID row without the old 64-row cap', async ({page}) => {
  const densityQueries = [];
  await page.route('**/api/query', async route => {
    const request = route.request();
    if (request.method() === 'POST') {
      const sql = request.postDataJSON().sql;
      if (sql.includes('tracks(group_order,group_name,track,first_ts)')) {
        const rows = [
          ...Array.from({length: 128}, (_, index) => ['cpu', index, 1.01]),
          ...Array.from({length: 128}, (_, index) => ['pid', 100 + index, 1.01]),
        ];
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({
            columns: ['group_name', 'track', 'first_ts'],
            rows,
            truncated: false,
            elapsed_ms: .1,
            sql,
          }),
        });
        return;
      }
      if (sql.startsWith('SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc FROM events') && sql.includes('ORDER BY ts,dur DESC LIMIT 10001')) {
        const rows = Array.from({length: 128}, (_, index) => [index + 1, 1.01, .001, 1.011, index, 100 + index, 0, 1024, `task.${index}`, 'user', 0, 0, 0]);
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({columns: [], rows, truncated: true, elapsed_ms: .1, sql}),
        });
        return;
      }
      if (sql.includes(' IN (') && (sql.includes('timeline_mipmap') || sql.includes('WITH RECURSIVE scoped'))) {
        densityQueries.push(sql);
      }
    }
    await route.continue();
  });
  await page.reload();
  await waitForTimeline(page);
  const timeline = page.locator('#timeline');
  const scroll = page.locator('#timeline-scroll');
  await expect(timeline).toHaveAttribute('data-track-count', '256');
  await expect(timeline).toHaveAttribute('data-track-catalog-truncated', 'false');
  expect((await timeline.getAttribute('data-visible-tracks')).split(',').length).toBeLessThan(64);
  expect(Number(await timeline.getAttribute('data-rendered-events'))).toBeLessThan(1000);

  await scroll.evaluate(element => element.scrollTop = 100 * 52);
  await expect(timeline.locator('.track-label[data-track="cpu:100"]')).toBeVisible();
  expect(Number(await timeline.getAttribute('data-y-start'))).toBeGreaterThan(80);
  await expect.poll(() => densityQueries.some(sql => /cpu IN \([^)]*\b100\b/.test(sql)))
    .toBe(true);

  await scroll.evaluate(element => element.scrollTop = 128 * 52);
  await expect(timeline.locator('.track-label[data-track="pid:100"]')).toBeVisible();
  await expect.poll(() => densityQueries.some(sql => /pid IN \([^)]*\b100\b/.test(sql)))
    .toBe(true);
  const beforeZoom = Number(await timeline.getAttribute('data-row-height'));
  await page.locator('#y-zoom-in').click();
  await expect.poll(async () => Number(await timeline.getAttribute('data-row-height')))
    .toBeGreaterThan(beforeZoom);
  await expect(timeline).toHaveAttribute('data-track-count', '256');
  const savedRowHeight = Number(await timeline.getAttribute('data-row-height'));
  const savedScroll = Number(await timeline.getAttribute('data-y-scroll'));
  await page.locator('#save-workspace').click();
  await page.reload();
  await waitForTimeline(page);
  await expect(timeline).toHaveAttribute('data-track-count', '256');
  await expect(timeline).toHaveAttribute('data-row-height', savedRowHeight.toFixed(3));
  await expect.poll(async () => Number(await timeline.getAttribute('data-y-scroll')))
    .toBeGreaterThan(savedScroll * .9);
});

test('discards stale timeline responses after a newer viewport/filter request', async ({page}) => {
  let armed = false;
  let slowStarted = 0;
  let newStarted = 0;
  await page.route('**/api/query', async route => {
    const request = route.request();
    if (armed && request.method() === 'POST') {
      const sql = request.postDataJSON().sql;
      const rawTimeline = sql.startsWith('SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc FROM events') && sql.includes('ORDER BY ts,dur DESC LIMIT 10001');
      if (rawTimeline && sql.includes("category = 'agent'") && !sql.includes("name LIKE")) {
        slowStarted++;
        await new Promise(resolve => setTimeout(resolve, 350));
      } else if (rawTimeline && sql.includes("name LIKE '%' || 'agent.tool' || '%'")) {
        newStarted++;
        await new Promise(resolve => setTimeout(resolve, 5));
      }
    }
    await route.continue();
  });
  armed = true;
  await page.locator('#filter-field').selectOption('category');
  await page.locator('#filter-value').fill('agent');
  await page.locator('#add-filter').click();
  await expect.poll(() => slowStarted).toBe(1);
  await page.locator('#filter-field').selectOption('name');
  await page.locator('#filter-op').selectOption('contains');
  await page.locator('#filter-value').fill('agent.tool');
  await page.locator('#add-filter').click();
  await expect.poll(() => newStarted).toBe(1);
  await waitForTimeline(page);
  await page.waitForTimeout(400);
  await expect(page.locator('#timeline')).toContainText('agent.tool');
  await expect(page.locator('#timeline')).not.toContainText('agent.reason');
});

test('keeps SQL, schema inspection, saved views, and portable workspace state', async ({page}) => {
  await page.locator('#filter-field').selectOption('category');
  await page.locator('#filter-value').fill('agent');
  await page.locator('#add-filter').click();
  await openDock(page, 'sql');
  await page.locator('#sql').fill('SELECT COUNT(*) AS agent_count FROM agent_spans');
  await page.locator('#run-sql').click();
  await expect(page.locator('#query-table th').first()).toHaveText('agent_count');
  await expect(page.locator('#query-table tbody td').first()).toHaveText('4');

  await page.locator('#view-name').fill('Agent spans');
  await page.locator('#save-view').click();
  await expect(page.locator('#saved-views')).toContainText('Agent spans');
  await page.locator('.track-label[data-track="cpu:0"]').click({modifiers: ['Shift']});
  await page.locator('[data-track-group="cpu"]').click();
  await waitForTimeline(page);
  await expect(page.locator('[data-track-group="cpu"]')).toHaveAttribute(
    'data-group-state',
    'highlighted',
  );
  await page.locator('#trace-search').fill('agent.tool');
  await page.locator('#search-min').fill('1000');
  await page.locator('#search-max').fill('5000');
  await page.locator('[data-view-slot="2"]').click({modifiers: ['Shift']});
  await page.locator('#save-workspace').click();
  await page.reload();
  await waitForTimeline(page);
  await expect(page.locator('#filter-chips')).toContainText('category = agent');
  await expect(page.locator('#saved-views')).toContainText('Agent spans');
  await expect(page.locator('#sql')).toHaveValue('SELECT COUNT(*) AS agent_count FROM agent_spans');
  await expect(page.locator('#timeline')).toHaveAttribute('data-highlighted-tracks', 'cpu:0');
  await expect(page.locator('[data-track-group="cpu"]')).toHaveAttribute(
    'data-group-state',
    'highlighted',
  );
  await expect(page.locator('#trace-search')).toHaveValue('agent.tool');
  await expect(page.locator('#search-min')).toHaveValue('1000');
  await expect(page.locator('#search-max')).toHaveValue('5000');
  await expect(page.locator('[data-view-slot="2"]')).toHaveAttribute('data-saved', 'true');

  const download = page.waitForEvent('download');
  await page.locator('#export-workspace').click();
  const exported = await download;
  const workspace = JSON.parse(await readFile(await exported.path(), 'utf8'));
  expect(workspace.kind).toBe('kutrace-workspace');
  expect(workspace.version).toBe(8);
  expect(workspace.trackGroups).toEqual({cpu: 'highlighted', pid: 'full', rpc: 'full', resource: 'full'});
  expect(workspace.highlightedTracks).toEqual(['cpu:0']);
  expect(workspace.rowHeight).toBe(52);
  expect(workspace.verticalScroll).toBe(0);
  expect(workspace.overlays).toEqual({
    marks: 3,
    arcs: 2,
    locks: 2,
    frequency: 2,
    ipc: 0,
    samples: 0,
    annotations: 0,
    colorblind: false,
  });
  expect(workspace.search).toEqual({
    text: 'agent.tool',
    minimum: '1000',
    maximum: '5000',
    units: 'microseconds',
    invert: false,
  });
  expect(workspace.viewSlots).toHaveLength(5);
  expect(workspace.viewSlots[0]).toBeNull();
  expect(workspace.viewSlots[2].search.text).toBe('agent.tool');
  expect(workspace.viewSlots[2].trackGroups.cpu).toBe('highlighted');
  expect(workspace.views).toEqual([{name: 'Agent spans', sql: 'SELECT COUNT(*) AS agent_count FROM agent_spans'}]);

  workspace.sql = 'SELECT name FROM profile_callchains LIMIT 3';
  await page.locator('#workspace-file').setInputFiles({
    name: 'workspace.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(workspace)),
  });
  await expect(page.locator('#query-status')).toContainText('Workspace imported');
  await expect(page.locator('#sql')).toHaveValue(workspace.sql);

  await page.locator('#show-schema').click();
  await expect(page.locator('#query-status')).toHaveText('sqlite_schema');
  await expect(page.locator('#query-table')).toContainText('profile_callchains');
});

test('shows an honest event fallback when sampled callchains are absent', async ({page}) => {
  await openDock(page, 'flamegraph');
  await expect(page.locator('#flame-status')).toContainText('no sampled callchains');
  await expect(page.locator('#flamegraph [data-flame-level="root"]')).toBeVisible();
  await expect(page.locator('#flamegraph [data-flame-level="name"]').first()).toBeVisible();
  await expect(page.locator('#flamegraph')).toContainText(/agent|runtime|write|getpid/i);
});

test('navigates agent spans and exposes their exact trace context to humans and agents', async ({page}) => {
  await openDock(page, 'agent');
  await expect(page.locator('#agent-tree [data-agent-span]')).toHaveCount(4);
  await page.locator('[data-agent-span="2"]').click();
  await expect(page.locator('#agent-context-title')).toContainText('agent.tool.read · span 2');
  await expect(page.locator('#agent-annotations')).toContainText('agent.observation.observed.read.delay');
  await expect(page.locator('#rpc-flows')).toContainText('RPC 77');
  await expect(page.locator('#resource-activity')).toContainText('RES 900');
  await expect(page.locator('#agent-context-table')).toContainText('ReadFile.77');
  await expect(page.locator('#agent-context-sql')).toContainText('WHERE ts <');
  await page.locator('#open-agent-context').click();
  await expect(page.locator('[data-dock="sql"]')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#sql')).toHaveValue(/SELECT ts,dur,cpu,pid,rpc,event,name/);
});

test('renders normalized symbolized callchains as a true sample flamegraph', async ({page}) => {
  await page.goto('http://127.0.0.1:38127/');
  await waitForTimeline(page);
  await openDock(page, 'flamegraph');
  await expect(page.locator('#flame-status')).toContainText('4 sampled stacks · depth 3');
  await expect(page.locator('#flamegraph')).toContainText('main');
  await expect(page.locator('#flamegraph')).toContainText('dispatch');
  await expect(page.locator('#flamegraph')).toContainText('parse_request');
  await expect(page.locator('#flamegraph')).toContainText('write_response');
  await expect(page.locator('#flamegraph')).toContainText('scheduler_tick');
  await expect(page.locator('[data-flame-level="callchain"]')).toHaveCount(7);
  await expect(page.locator('#flame-weight')).toBeDisabled();
});

test('preserves the original standalone KUtrace compatibility route', async ({page}) => {
  const source = await readFile('../../../hello_world_demo_live.html');
  const response = await page.request.get('/legacy');
  expect(response.status()).toBe(200);
  const served = await response.body();
  expect(served.subarray(0, source.length).equals(source)).toBe(true);

  await page.goto('/legacy');
  await expect(page.getByRole('button', {name: 'Mark'})).toBeVisible();
  const mark = page.locator('#showmarks');
  const before = await mark.evaluate(button => button.style.backgroundColor);
  await mark.click();
  await expect.poll(() => mark.evaluate(button => button.style.backgroundColor)).not.toBe(before);
  const initial = await page.evaluate(() => [window.realxleft, window.realxright]);
  await page.locator('body').evaluate(body => { body.tabIndex = -1; body.focus(); });
  await page.keyboard.press('KeyW');
  await expect.poll(() => page.evaluate(() => window.realxright - window.realxleft)).toBeLessThan(initial[1] - initial[0]);
  await page.keyboard.press('Digit0');
  await expect.poll(() => page.evaluate(() => [window.realxleft, window.realxright])).toEqual(initial);
});
