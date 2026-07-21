import {expect, test} from '@playwright/test';
import {readFile} from 'node:fs/promises';

async function openDock(page, name) {
  await page.locator(`[data-dock="${name}"]`).click();
  await expect(page.locator(`[data-dock="${name}"]`)).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator(`[data-dock-panel="${name}"]`)).toBeVisible();
}

async function dragTimelineRange(page, from = .3, to = .7) {
  const timeline = page.locator('#timeline');
  const box = await timeline.boundingBox();
  expect(box).not.toBeNull();
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * from, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * to, y, {steps: 5});
  await page.mouse.up();
  return box;
}

test.beforeEach(async ({page}) => {
  const response=await page.goto('/');
  expect(response.headers()['cache-control']).toBe('no-store');
  await expect(page.locator('#trace-title')).toContainText('Agent reasoning fixture');
  await expect(page.locator('#event-count')).toContainText('18 rows');
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready', 'true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source', 'events');
  await expect(page.locator('#timeline')).toHaveAttribute('data-detail', 'true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-renderer','kutrace');
  await expect(page.locator('#cpu-controls')).toBeHidden();
  await expect(page.locator('.renderer-tab[data-renderer="kutrace"]')).toHaveAttribute('aria-selected','true');
  await expect(page.locator('#timeline')).toBeVisible();
});

test('switches between the continuous timeline and exact KUtrace in app', async ({page, browserName}) => {
  const timelineTab = page.locator('[data-view="timeline"]');
  const legacyTab = page.locator('[data-view="legacy"]');
  await expect(timelineTab).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#timeline-view')).toBeVisible();
  await expect(page.locator('#legacy-view')).toBeHidden();
  await expect(page.locator('#legacy-frame')).not.toHaveAttribute('src', /.+/);
  await page.locator('.renderer-tab[data-renderer="lanes"]').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-renderer','lanes');
  await expect(page.locator('#renderer-label')).toHaveText('Event lanes');
  await page.locator('.renderer-tab[data-renderer="kutrace"]').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-renderer','kutrace');
  await expect(page.locator('#renderer-label')).toHaveText('Native KUtrace');

  await legacyTab.click();
  await expect(legacyTab).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#timeline-view')).toBeHidden();
  await expect(page.locator('#legacy-view')).toBeVisible();
  await expect(page.locator('#legacy-frame')).toHaveAttribute('src', '/legacy');
  const legacy=page.locator('#legacy-frame').contentFrame();
  await expect(legacy.getByRole('button', {name: 'Mark'})).toBeVisible();
  const initialRange=await legacy.locator('body').evaluate(()=>[window.realxleft,window.realxright]);
  await legacy.locator('body').evaluate(body=>{body.tabIndex=-1;body.focus()});
  await page.keyboard.press('KeyW');
  await expect.poll(()=>legacy.locator('body').evaluate(()=>window.realxright-window.realxleft)).toBeLessThan(initialRange[1]-initialRange[0]);
  const zoomedLeft=await legacy.locator('body').evaluate(()=>window.realxleft);
  await page.keyboard.press('KeyD');
  await expect.poll(()=>legacy.locator('body').evaluate(()=>window.realxleft)).toBeGreaterThan(zoomedLeft);
  await page.keyboard.press('Digit0');
  await expect.poll(()=>legacy.locator('body').evaluate(()=>[window.realxleft,window.realxright])).toEqual(initialRange);

  await timelineTab.click();
  await expect(timelineTab).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#timeline')).toBeVisible();
  if (browserName === 'chromium') {
    await expect(page).toHaveScreenshot('workspace.png', {
      animations: 'disabled',
      fullPage: true,
      mask: [page.locator('#query-status')],
      maskColor: '#121722'
    });
  }
});

test('keeps agent reasoning in an explicit analysis dock', async ({page}) => {
  await expect(page.locator('[data-dock-panel="agent"]')).toBeHidden();
  await openDock(page, 'agent');
  await expect(page.locator('#agent-tree')).toContainText('agent.reason');
  await expect(page.locator('#agent-tree')).toContainText('agent.tool.read');
  await expect(page.locator('#agent-tree')).toContainText('agent.tool.write');
  await expect(page.locator('[data-agent-span="1"]')).toContainText('3 notes');
  await expect(page.locator('#agent-context-title')).toContainText('agent.reason · span #1');
  await expect(page.locator('#agent-annotations')).toContainText('query agent.query.SELECT.events.latency · 11');
  await expect(page.locator('#agent-annotations')).toContainText('decision agent.decision.retry.write · 33');
  await expect(page.locator('#agent-annotations')).toContainText('result agent.result.success · 44');
  await expect(page.locator('#agent-annotations')).not.toContainText('ordinary.mark');
  await expect(page.locator('#event-table')).toContainText('ordinary.mark');
  await expect(page.locator('#agent-context-sql')).toContainText('FROM agent_annotations');
  await page.locator('[data-agent-span="2"]').click();
  await expect(page.locator('#agent-context-title')).toContainText('agent.tool.read · span #2');
  await expect(page.locator('#agent-annotations')).toContainText('observation agent.observation.observed.read.delay · 22');
  await page.locator('[data-agent-span="1"]').click();
  await expect(page.locator('#rpc-flows')).toContainText('RPC 77');
  await expect(page.locator('#rpc-flows')).toContainText('ReadFile.77');
  await expect(page.locator('#resource-activity')).toContainText('resource.database');
  await expect(page.locator('#resource-activity')).toContainText('enqueue.worker');
  await page.locator('#open-agent-context').click();
  await expect(page.locator('[data-dock="sql"]')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#sql')).toHaveValue(/WHERE span_id=1/);
  await expect(page.locator('#query-status')).toContainText('rows');
  await page.locator('#view-name').fill('Root agent inspection');
  await page.locator('#save-view').click();
  await expect(page.locator('#saved-views')).toContainText('Root agent inspection');
  await expect(page.locator('#query-status')).toContainText('Saved view');
  await expect(page.locator('#event-table')).toContainText('IPC 2.5 · LLC 512');
  await expect(page.locator('#perf-legend')).toContainText('IPC');
  await expect(page.locator('#perf-legend')).toContainText('LLC');
  await expect(page.locator('a[href="/legacy"]').first()).toHaveText(/Exact KUtrace view/);
  await expect(page.locator('#timeline')).toBeVisible();
  await expect(page.locator('#timeline-mode')).toContainText('Native KUtrace');
  const timelineDimensions=await page.locator('#timeline').evaluate(canvas=>({height:canvas.getBoundingClientRect().height,viewport:canvas.closest('.timeline-scroll').clientHeight}));
  expect(timelineDimensions.height).toBeGreaterThanOrEqual(timelineDimensions.viewport);
});

test('composes filters, runs SQL, zooms, and restores saved state', async ({page}) => {
  await page.locator('#filter-field').selectOption('category');
  await page.locator('#filter-op').selectOption('=');
  await page.locator('#filter-value').fill('agent');
  await page.locator('#add-filter').click();
  await expect(page.locator('#filter-chips')).toContainText('category = agent');
  await expect(page.locator('#event-count')).toContainText('4 rows');

  await openDock(page, 'sql');
  await page.locator('#sql').fill('SELECT COUNT(*) AS agent_count FROM agent_spans');
  await page.locator('#run-sql').click();
  await expect(page.locator('#query-table tbody td').first()).toHaveText('4');
  await expect(page.locator('#query-status')).toContainText('1 rows');

  const initialRange = await page.locator('#range-label').textContent();
  await page.locator('#zoom-in').click();
  await expect(page.locator('#range-label')).not.toHaveText(initialRange);
  await page.keyboard.press('Digit0');
  await expect(page.locator('#range-label')).toHaveText(initialRange);
  await page.keyboard.press('KeyW');
  await expect(page.locator('#range-label')).not.toHaveText(initialRange);
  const zoomedRange = await page.locator('#range-label').textContent();
  await page.keyboard.press('KeyD');
  await expect(page.locator('#range-label')).not.toHaveText(zoomedRange);
  const pannedRange = await page.locator('#range-label').textContent();
  await page.locator('#sql').focus();
  await page.keyboard.press('ArrowLeft');
  await expect(page.locator('#range-label')).toHaveText(pannedRange);
  await page.locator('#timeline').focus();
  await page.keyboard.press('KeyS');
  await expect(page.locator('#range-label')).not.toHaveText(pannedRange);
  await page.keyboard.press('Home');
  await expect(page.locator('#range-label')).toHaveText(initialRange);

  const immediate = await page.evaluate(() => {
    const before=document.querySelector('#range-label').textContent;
    window.dispatchEvent(new KeyboardEvent('keydown',{key:'w',bubbles:true}));
    const result={before,after:document.querySelector('#range-label').textContent,preview:document.querySelector('#timeline').dataset.preview};
    window.dispatchEvent(new KeyboardEvent('keyup',{key:'w',bubbles:true}));
    return result;
  });
  expect(immediate.after).not.toBe(immediate.before);
  expect(immediate.preview).toBe('true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await page.keyboard.press('Home');

  await page.keyboard.down('KeyW');
  await page.waitForTimeout(90);
  const heldRange=await page.locator('#range-label').textContent();
  await page.waitForTimeout(90);
  await expect(page.locator('#range-label')).not.toHaveText(heldRange);
  await expect(page.locator('#timeline')).toHaveAttribute('data-preview','true');
  await page.keyboard.up('KeyW');
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await page.keyboard.press('Home');

  await page.locator('#save-workspace').click();
  await page.reload();
  await expect(page.locator('#filter-chips')).toContainText('category = agent');
  await expect(page.locator('#sql')).toHaveValue('SELECT COUNT(*) AS agent_count FROM agent_spans');
});

test('prints capture provenance, visible range, filters, and the track legend', async ({page}) => {
  await page.locator('#filter-field').selectOption('category');
  await page.locator('#filter-value').fill('agent');
  await page.locator('#add-filter').click();
  await page.locator('#zoom-in').click();
  const visibleRange = await page.locator('#range-label').textContent();
  await page.emulateMedia({media: 'print'});
  await expect(page.locator('.print-summary')).toBeVisible();
  await expect(page.locator('#print-trace-title')).toContainText('Agent reasoning fixture');
  await expect(page.locator('#print-range')).toContainText(visibleRange.split(' · ')[0]);
  await expect(page.locator('#print-filters')).toContainText('category = agent');
  await expect(page.locator('header')).toBeHidden();
  await expect(page.locator('aside')).toBeHidden();
  await expect(page.locator('.sql-card')).toBeHidden();
  await expect(page.locator('.legend')).toBeVisible();
  await expect(page.locator('#timeline')).toBeVisible();
});

test('uses a collapsible dock for details, flamegraph, SQL, and agent analysis', async ({page}) => {
  for (const name of ['details', 'flamegraph', 'sql', 'agent']) {
    await openDock(page, name);
    for (const other of ['details', 'flamegraph', 'sql', 'agent'].filter(value => value !== name)) {
      await expect(page.locator(`[data-dock-panel="${other}"]`)).toBeHidden();
    }
  }

  await page.locator('#toggle-dock').click();
  await expect(page.locator('.dock-body')).toBeHidden();
  await page.locator('#toggle-dock').click();
  await expect(page.locator('.dock-body')).toBeVisible();
});

test('keeps the overview viewport synchronized with zoom and pan', async ({page}) => {
  const viewport = page.locator('#overview-viewport');
  await expect(page.locator('#overview')).toBeVisible();
  expect(await page.locator('#overview').evaluate(canvas => canvas.width)).toBeGreaterThan(0);
  await expect(viewport).toBeVisible();
  const initial = await viewport.evaluate(element => ({left: element.style.left, width: element.style.width}));

  await page.locator('#zoom-in').click();
  await expect.poll(() => viewport.evaluate(element => element.style.width)).not.toBe(initial.width);
  const zoomed = await viewport.evaluate(element => ({left: element.style.left, width: element.style.width}));
  expect(Number.parseFloat(zoomed.width)).toBeLessThan(Number.parseFloat(initial.width));

  await page.locator('#pan-right').click();
  await expect.poll(() => viewport.evaluate(element => element.style.left)).not.toBe(zoomed.left);
});

test('builds a range-scoped flamegraph with clickable frames', async ({page}) => {
  await dragTimelineRange(page, .2, .8);
  await openDock(page, 'flamegraph');
  await expect(page.locator('#flame-range')).not.toBeEmpty();
  await expect(page.locator('#flame-status')).toContainText(/frame|span/i);
  const frames = page.locator('#flamegraph .flame-frame');
  await expect(frames.first()).toBeVisible();
  await expect(page.locator('#flamegraph')).toContainText(/agent|runtime|write|getpid/i);

  const rangeBefore = await page.locator('#range-label').textContent();
  await frames.first().click();
  await expect(page.locator('#range-label')).not.toHaveText(rangeBefore);
});

test('shows auto-pruned KUtrace CPU, PID, RPC, and resource groups', async ({page}) => {
  const groupingQueries=[];
  await page.route('**/api/query',async route=>{const request=route.request();if(request.method()==='POST'){const sql=request.postDataJSON().sql;if(sql.startsWith('SELECT DISTINCT cpu FROM events')||sql.startsWith('SELECT DISTINCT pid FROM events'))groupingQueries.push(sql)}await route.continue()});
  await page.reload();
  const timeline = page.locator('#timeline');
  await expect(timeline).toHaveAttribute('data-track-mode', 'cpu_pid');
  await expect(timeline).toHaveAttribute('data-track-groups', 'cpu,pid,rpc,res');
  await expect(timeline).toHaveAttribute('data-visible-tracks',/.+/);
  await expect(page.locator('#track-mode')).toHaveValue('cpu_pid');
  expect(groupingQueries.find(sql=>sql.startsWith('SELECT DISTINCT cpu'))).toContain('dur>0 AND pid>0 AND cpu>=0');
  expect(groupingQueries.find(sql=>sql.startsWith('SELECT DISTINCT pid'))).toContain('dur>0 AND pid>0 AND cpu>=0');
  const linkedTracks=(await timeline.getAttribute('data-visible-tracks')).split(',');
  expect(linkedTracks.some(track=>track.startsWith('cpu:'))).toBe(true);
  expect(linkedTracks.some(track=>track.startsWith('pid:'))).toBe(true);
  expect(linkedTracks.some(track=>track.startsWith('rpc:'))).toBe(true);
  expect(linkedTracks.some(track=>track.startsWith('res:'))).toBe(true);
  await page.locator('[data-track-group="process"]').click();
  await expect(timeline).toHaveAttribute('data-ready','true');
  await expect.poll(async()=>timeline.getAttribute('data-visible-tracks')).not.toContain('cpu:');
  await expect.poll(async()=>timeline.getAttribute('data-visible-tracks')).not.toContain('pid:');
  await page.locator('[data-track-group="process"]').click();
  await expect(timeline).toHaveAttribute('data-ready','true');
  await page.locator('#track-mode').selectOption('pid');
  await expect(timeline).toHaveAttribute('data-ready', 'true');
  await expect(timeline).toHaveAttribute('data-track-mode', 'pid');
  await expect(timeline).toHaveAttribute('data-track-groups', 'pid');
  await expect(page.locator('#cpu-controls')).toBeHidden();
  await page.locator('#track-mode').selectOption('cpu');
  await expect(timeline).toHaveAttribute('data-track-mode', 'cpu');
  await expect(timeline).toHaveAttribute('data-track-groups', 'cpu');
  await page.locator('#track-mode').selectOption('cpu_pid');
  await expect(timeline).toHaveAttribute('data-track-groups', 'cpu,pid,rpc,res');
});

test('shift click toggles a visible track highlight', async ({page}) => {
  const timeline=page.locator('#timeline');
  const point=await page.evaluate(()=>{const hit=state.hitRegions.find(region=>String(region.track).startsWith('cpu:'));return {x:hit.x+Math.max(1,hit.w/2),y:hit.y+Math.max(1,hit.h/2),track:String(hit.track)}});
  const box=await timeline.boundingBox();
  await page.keyboard.down('Shift');
  await page.mouse.click(box.x+point.x,box.y+point.y);
  await page.keyboard.up('Shift');
  await expect(timeline).toHaveAttribute('data-highlighted-tracks',point.track);
  await page.keyboard.down('Shift');
  await page.mouse.click(box.x+point.x,box.y+point.y);
  await page.keyboard.up('Shift');
  await expect(timeline).toHaveAttribute('data-highlighted-tracks','');
});

test('searches the visible trace, inverts matches, and toggles KUtrace overlays', async ({page}) => {
  await page.locator('#trace-search').fill('agent.tool');
  await expect(page.locator('#search-count')).toHaveText(/2\s+(matches|events)/i);
  await expect(page.locator('#timeline')).toHaveAttribute('data-search-count', '2');
  await page.locator('#search-invert').click();
  await expect(page.locator('#search-invert')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#search-count')).toHaveText(/16\s+(matches|events)/i);

  for (const overlay of ['marks', 'arcs', 'locks', 'frequency', 'ipc', 'samples']) {
    const button = page.locator(`[data-overlay="${overlay}"]`);
    await expect(button).toHaveAttribute('aria-pressed', 'true');
    await button.click();
    await expect(button).toHaveAttribute('aria-pressed', 'false');
  }
  const colorblind = page.locator('[data-overlay="colorblind"]');
  await colorblind.click();
  await expect(colorblind).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('html')).toHaveClass(/colorblind/);
});

test('escapes filter editing back to timeline navigation', async ({page}) => {
  const filter=page.locator('#filter-value'),timeline=page.locator('#timeline');
  await filter.fill('agent.tool');
  await filter.focus();
  const initialRange=await page.locator('#range-label').textContent();
  await page.keyboard.press('KeyW');
  await expect(page.locator('#range-label')).toHaveText(initialRange);
  const editedValue=await filter.inputValue();
  await page.keyboard.press('Escape');
  await expect(timeline).toBeFocused();
  await expect(filter).toHaveValue(editedValue);
  await page.keyboard.press('KeyW');
  await expect(page.locator('#range-label')).not.toHaveText(initialRange);
});

test('follows a growing trace tail while continuing passive extent polling', async ({page}) => {
  let extentQueries = 0;
  await page.route('**/api/query', async route => {
    const request = route.request();
    if (request.method() === 'POST' && request.postDataJSON().sql === 'SELECT COUNT(*),MIN(ts),MAX(ts_end) FROM events') {
      extentQueries++;
      const end = extentQueries === 1 ? 1.050 : 1.080;
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({columns: ['COUNT(*)', 'MIN(ts)', 'MAX(ts_end)'], rows: [[18 + extentQueries, 1, end]], truncated: false, elapsed_ms: .1}),
      });
      return;
    }
    await route.continue();
  });
  await page.reload();
  await expect(page.locator('#follow-live')).toHaveAttribute('aria-pressed', 'false');
  await page.locator('#follow-live').click();
  await expect(page.locator('#follow-live')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('#live-status')).toContainText(/following|live/i);
  await expect.poll(() => extentQueries, {timeout: 5000}).toBeGreaterThan(0);
  await expect(page.locator('#range-label')).toContainText('1.050000s');

  await page.locator('#follow-live').click();
  await expect(page.locator('#follow-live')).toHaveAttribute('aria-pressed', 'false');
  const stoppedRange = await page.locator('#range-label').textContent();
  await expect.poll(() => extentQueries, {timeout: 5000}).toBeGreaterThan(1);
  await expect(page.locator('#range-label')).toHaveText(stoppedRange);
});

test('keeps an area selection until it is zoomed or cleared', async ({page}) => {
  const timeline = page.locator('#timeline');
  const initialRange = await page.locator('#range-label').textContent();
  const box = await dragTimelineRange(page);
  await expect(page.locator('#time-selection')).toBeVisible();
  await expect(page.locator('#zoom-selection')).toBeEnabled();
  await expect(page.locator('#clear-selection')).toBeEnabled();
  await expect(page.locator('#range-label')).toHaveText(initialRange);
  await expect(page.locator('#selection-summary')).not.toContainText('Select an event');

  await page.locator('#zoom-selection').click();
  await expect(page.locator('#range-label')).not.toHaveText(initialRange);
  await expect(page.locator('#time-selection')).toBeVisible();

  const selectedRange = await page.locator('#range-label').textContent();
  await page.mouse.move(box.x + box.width * .6, box.y + box.height / 2);
  await page.keyboard.down('Control');
  await page.mouse.wheel(0, -180);
  await page.keyboard.up('Control');
  await expect(page.locator('#range-label')).not.toHaveText(selectedRange);

  const zoomedRange = await page.locator('#range-label').textContent();
  await page.keyboard.down('Shift');
  await page.mouse.wheel(0, 120);
  await page.keyboard.up('Shift');
  await expect(page.locator('#range-label')).not.toHaveText(zoomedRange);

  await dragTimelineRange(page, .4, .6);
  await expect(page.locator('#time-selection')).toBeVisible();
  await page.locator('#clear-selection').click();
  await expect(page.locator('#time-selection')).toBeHidden();
  await expect(page.locator('#zoom-selection')).toBeDisabled();
});

test('virtualizes machines with more than 64 CPU tracks at the SQL boundary', async ({page}) => {
  const timelineQueries=[];
  await page.route('**/api/query', async route => {
    const request=route.request();
    if(request.method()==='POST'){
      const payload=request.postDataJSON();
      if(payload.sql.startsWith('SELECT DISTINCT cpu FROM events')){
        await route.fulfill({contentType:'application/json',body:JSON.stringify({columns:['cpu'],rows:Array.from({length:66},(_,cpu)=>[cpu]),truncated:false,elapsed_ms:0.1})});
        return;
      }
      if(payload.sql.includes('WITH RECURSIVE scoped_events')||payload.sql.startsWith('SELECT id,ts,dur,ts_end,cpu'))timelineQueries.push(payload.sql);
    }
    await route.continue();
  });
  await page.reload();
  await page.locator('#track-mode').selectOption('cpu');
  await page.locator('.renderer-tab[data-renderer="lanes"]').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#cpu-controls')).toBeVisible();
  await expect(page.locator('#cpu-window')).toHaveText('CPUs 1–64 of 66');
  await expect(page.locator('#previous-cpus')).toBeDisabled();
  await expect(page.locator('#next-cpus')).toBeEnabled();
  expect(timelineQueries.at(-1)).toContain('cpu IN (0,1,2');
  expect(timelineQueries.at(-1)).toContain(',63)');
  expect(timelineQueries.at(-1)).not.toContain(',64,65)');

  await page.locator('#next-cpus').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#cpu-window')).toHaveText('CPUs 65–66 of 66');
  await expect(page.locator('#previous-cpus')).toBeEnabled();
  await expect(page.locator('#next-cpus')).toBeDisabled();
  expect(timelineQueries.at(-1)).toContain('cpu IN (64,65)');
});

test('uses the mipmap at low zoom and exact events for non-materialized filters', async ({page}) => {
  const timelineQueries=[];let wideCpuCount=0;
  await page.route('**/api/query', async route => {
    const request=route.request();
    if(request.method()==='POST'){
      const payload=request.postDataJSON();
      if(payload.sql==='SELECT COUNT(*),MIN(ts),MAX(ts_end) FROM events'){
        await route.fulfill({contentType:'application/json',body:JSON.stringify({columns:['COUNT(*)','MIN(ts)','MAX(ts_end)'],rows:[[18,0,10]],truncated:false,elapsed_ms:0.1})});
        return;
      }
      if(wideCpuCount&&payload.sql.startsWith('SELECT DISTINCT cpu FROM events')){
        await route.fulfill({contentType:'application/json',body:JSON.stringify({columns:['cpu'],rows:Array.from({length:wideCpuCount},(_,cpu)=>[cpu]),truncated:false,elapsed_ms:0.1})});
        return;
      }
      if((payload.sql.includes('ranked AS')&&payload.sql.includes('SELECT bucket,cpu,event'))||payload.sql.startsWith('SELECT id,ts,dur,ts_end,cpu'))timelineQueries.push(payload.sql);
    }
    await route.continue();
  });
  await page.reload();
  await page.locator('#track-mode').selectOption('cpu');
  await page.locator('.renderer-tab[data-renderer="lanes"]').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source','mipmap');
  await expect(page.locator('#timeline')).toHaveAttribute('data-mipmap-level','fine');
  expect(timelineQueries.at(-1)).toContain('FROM timeline_mipmap');
  expect(timelineQueries.at(-1)).toContain('long_events');

  wideCpuCount=64;
  await page.reload();
  await page.locator('#track-mode').selectOption('cpu');
  await page.locator('.renderer-tab[data-renderer="lanes"]').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-mipmap-level','coarse');
  expect(timelineQueries.at(-1)).toContain('FROM timeline_mipmap_coarse');

  await page.locator('#filter-field').selectOption('category');
  await page.locator('#filter-op').selectOption('=');
  await page.locator('#filter-value').fill('agent');
  await page.locator('#add-filter').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source','mipmap');
  expect(timelineQueries.at(-1)).toContain("category = 'agent'");

  await page.locator('#filter-field').selectOption('name');
  await page.locator('#filter-op').selectOption('contains');
  await page.locator('#filter-value').fill('agent.');
  await page.locator('#add-filter').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source','events');
  expect(timelineQueries.at(-1)).toContain("name LIKE '%' || 'agent.' || '%'");

  await page.locator('#filter-field').selectOption('pid');
  await page.locator('#filter-op').selectOption('=');
  await page.locator('#filter-value').fill('100');
  await page.locator('#add-filter').click();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source','events');
  expect(timelineQueries.at(-1)).toContain('FROM events');
  expect(timelineQueries.at(-1)).toContain('pid = 100');
});

test('serves the original legacy viewer with keyboard navigation', async ({page}) => {
  const source=await readFile('../../../hello_world_demo_live.html');
  const response=await page.request.get('/legacy');
  expect(response.status()).toBe(200);
  const served=await response.body();
  expect(served.subarray(0,source.length).equals(source)).toBe(true);
  expect(served.subarray(source.length).toString()).toContain('/legacy-keyboard.js');
  await page.goto('/legacy');
  await expect(page.getByRole('button', {name: 'Mark'})).toBeVisible();
  await expect(page.locator('body')).toContainText('hello world');
  await expect(page.locator('svg').first()).toBeVisible();
  const mark=page.locator('#showmarks'),colorBlind=page.locator('#showcb');
  const markBefore=await mark.evaluate(button=>button.style.backgroundColor);
  const colorBlindBefore=await colorBlind.evaluate(button=>button.style.backgroundColor);
  await mark.click();
  await expect.poll(()=>mark.evaluate(button=>button.style.backgroundColor)).not.toBe(markBefore);
  await colorBlind.click();
  await expect.poll(()=>colorBlind.evaluate(button=>button.style.backgroundColor)).not.toBe(colorBlindBefore);

  const annotateUser=page.locator('#annotateuser'),annotateAll=page.locator('#annotateall');
  const userBefore=await annotateUser.evaluate(button=>button.style.backgroundColor);
  const allBefore=await annotateAll.evaluate(button=>button.style.backgroundColor);
  await annotateUser.click();
  await expect.poll(()=>annotateUser.evaluate(button=>button.style.backgroundColor)).not.toBe(userBefore);
  await annotateAll.click();
  await expect.poll(()=>annotateAll.evaluate(button=>button.style.backgroundColor)).not.toBe(allBefore);
  await expect.poll(()=>annotateUser.evaluate(button=>button.style.backgroundColor)).toBe(userBefore);

  const search=page.locator('#SearchText');
  await search.fill('');
  await search.pressSequentially('hello_world_tra');
  await expect(page.locator('#matchcount')).toHaveText(/Matches: [1-9][0-9]*/);
  await expect(search).toHaveCSS('background-color','rgb(240, 240, 255)');
  const invert=page.locator('#searchnot'),invertBefore=await invert.evaluate(button=>button.style.backgroundColor);
  await invert.click();
  await expect.poll(()=>invert.evaluate(button=>button.style.backgroundColor)).not.toBe(invertBefore);

  const initialRange=await page.evaluate(()=>[window.realxleft,window.realxright]);
  await page.locator('body').evaluate(body=>{body.tabIndex=-1;body.focus()});
  await page.keyboard.press('KeyW');
  await expect.poll(async()=>page.evaluate(()=>window.realxright-window.realxleft)).toBeLessThan(initialRange[1]-initialRange[0]);
  const keyboardZoomedLeft=await page.evaluate(()=>window.realxleft);
  await page.keyboard.press('KeyD');
  await expect.poll(async()=>page.evaluate(()=>window.realxleft)).toBeGreaterThan(keyboardZoomedLeft);
  await page.keyboard.press('Digit0');
  await expect.poll(async()=>page.evaluate(()=>[window.realxleft,window.realxright])).toEqual(initialRange);
  const zoomSurface=await page.locator('#panzoomrect_x').boundingBox();
  expect(zoomSurface).not.toBeNull();
  await page.mouse.move(zoomSurface.x+zoomSurface.width/2,zoomSurface.y+zoomSurface.height/2);
  await page.mouse.wheel(0,-600);
  await expect.poll(async()=>page.evaluate(()=>window.realxright-window.realxleft)).toBeLessThan(initialRange[1]-initialRange[0]);
  await page.locator('#reddot').click();
  await expect.poll(async()=>page.evaluate(()=>[window.realxleft,window.realxright])).toEqual(initialRange);
});

test('exports, validates, and imports portable workspaces with named SQL views', async ({page}) => {
  await page.locator('#filter-field').selectOption('name');
  await page.locator('#filter-op').selectOption('contains');
  await page.locator('#filter-value').fill('agent.');
  await page.locator('#add-filter').click();
  await openDock(page, 'sql');
  await page.locator('#sql').fill('SELECT name, dur FROM agent_spans ORDER BY dur DESC');
  await page.locator('#view-name').fill('Slow agent spans');
  await page.locator('#save-view').click();
  const download = page.waitForEvent('download');
  await page.locator('#export-workspace').click();
  const exported = await download;
  const content = JSON.parse(await readFile(await exported.path(), 'utf8'));
  expect(content.kind).toBe('kutrace-workspace');
  expect(content.version).toBe(2);
  expect(content.filters).toEqual([{field: 'name', op: 'contains', value: 'agent.'}]);
  expect(content.views).toEqual([{name: 'Slow agent spans', sql: 'SELECT name, dur FROM agent_spans ORDER BY dur DESC'}]);

  content.sql = 'SELECT COUNT(*) AS imported_count FROM events';
  await page.locator('#workspace-file').setInputFiles({
    name: 'workspace.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(content)),
  });
  await expect(page.locator('#query-status')).toContainText('Workspace imported');
  await expect(page.locator('#sql')).toHaveValue(content.sql);
  await expect(page.locator('#filter-chips')).toContainText('name contains agent.');
  await expect(page.locator('#saved-views')).toContainText('Slow agent spans');
  await page.locator('[data-load-view="0"]').click();
  await expect(page.locator('#sql')).toHaveValue('SELECT name, dur FROM agent_spans ORDER BY dur DESC');
  await expect(page.locator('#query-table')).toContainText('agent.reason');

  const legacyWorkspace={kind:'kutrace-workspace',version:1,filters:[],sql:'SELECT 7 AS legacy_workspace'};
  await page.locator('#workspace-file').setInputFiles({
    name: 'workspace-v1.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(legacyWorkspace)),
  });
  await expect(page.locator('#query-status')).toContainText('Workspace imported');
  await expect(page.locator('#sql')).toHaveValue(legacyWorkspace.sql);
  await expect(page.locator('#saved-views')).toContainText('No saved SQL views');

  await page.locator('#workspace-file').setInputFiles({
    name: 'invalid.json',
    mimeType: 'application/json',
    buffer: Buffer.from('{"kind":"not-kutrace","version":1}'),
  });
  await expect(page.locator('#query-status')).toContainText('Unsupported workspace file');

  await page.locator('#workspace-file').setInputFiles({
    name: 'invalid-views.json',
    mimeType: 'application/json',
    buffer: Buffer.from('{"kind":"kutrace-workspace","version":2,"views":[{"name":"","sql":"SELECT 1"}]}'),
  });
  await expect(page.locator('#query-status')).toContainText('Invalid saved SQL views');
});
