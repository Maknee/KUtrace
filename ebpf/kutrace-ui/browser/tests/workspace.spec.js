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

  for (const overlay of ['marks', 'arcs', 'locks', 'frequency', 'ipc', 'samples', 'colorblind']) {
    const button = page.locator(`[data-overlay="${overlay}"]`);
    const before = await button.getAttribute('aria-pressed');
    await button.click();
    await expect(button).toHaveAttribute('aria-pressed', before === 'true' ? 'false' : 'true');
  }

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

test('caps CPU and PID rows independently in combined mode', async ({page}) => {
  await page.route('**/api/query', async route => {
    const request = route.request();
    if (request.method() === 'POST') {
      const sql = request.postDataJSON().sql;
      if (sql.startsWith('SELECT id,ts,dur,ts_end,cpu,pid,rpc,event,name,category,arg0,retval,ipc FROM events') && sql.includes('ORDER BY ts,dur DESC LIMIT 10001')) {
        const rows = Array.from({length: 64}, (_, index) => [index + 1, 1.01, .001, 1.011, index, 100 + index, 0, 1024, `task.${index}`, 'user', 0, 0, 0]);
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
  const tracks = (await page.locator('#timeline').getAttribute('data-visible-tracks')).split(',');
  expect(tracks.filter(track => track.startsWith('cpu:'))).toHaveLength(64);
  expect(tracks.filter(track => track.startsWith('pid:'))).toHaveLength(64);
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
  await page.locator('#save-workspace').click();
  await page.reload();
  await waitForTimeline(page);
  await expect(page.locator('#filter-chips')).toContainText('category = agent');
  await expect(page.locator('#saved-views')).toContainText('Agent spans');
  await expect(page.locator('#sql')).toHaveValue('SELECT COUNT(*) AS agent_count FROM agent_spans');

  const download = page.waitForEvent('download');
  await page.locator('#export-workspace').click();
  const exported = await download;
  const workspace = JSON.parse(await readFile(await exported.path(), 'utf8'));
  expect(workspace.kind).toBe('kutrace-workspace');
  expect(workspace.version).toBe(2);
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
