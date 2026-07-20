import {expect, test} from '@playwright/test';
import {readFile} from 'node:fs/promises';

test.beforeEach(async ({page}) => {
  await page.goto('/');
  await expect(page.locator('#trace-title')).toContainText('Agent reasoning fixture');
  await expect(page.locator('#event-count')).toContainText('18 rows');
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready', 'true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source', 'events');
  await expect(page.locator('#cpu-controls')).toBeHidden();
});

test('renders query-backed tracks and agent relationships', async ({page, browserName}) => {
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
  await page.locator('#open-agent-context').click();
  await expect(page.locator('#sql')).toHaveValue(/WHERE span_id=1/);
  await expect(page.locator('#query-status')).toContainText('rows');
  await page.locator('#view-name').fill('Root agent inspection');
  await page.locator('#save-view').click();
  await expect(page.locator('#saved-views')).toContainText('Root agent inspection');
  await expect(page.locator('#query-status')).toContainText('Saved view');
  await page.locator('[data-agent-span="2"]').click();
  await expect(page.locator('#agent-context-title')).toContainText('agent.tool.read · span #2');
  await expect(page.locator('#agent-annotations')).toContainText('observation agent.observation.observed.read.delay · 22');
  await page.locator('[data-agent-span="1"]').click();
  await expect(page.locator('#rpc-flows')).toContainText('RPC 77');
  await expect(page.locator('#rpc-flows')).toContainText('ReadFile.77');
  await expect(page.locator('#resource-activity')).toContainText('resource.database');
  await expect(page.locator('#resource-activity')).toContainText('enqueue.worker');
  await expect(page.locator('#event-table')).toContainText('IPC 2.5 · LLC 512');
  await expect(page.locator('#perf-legend')).toContainText('IPC');
  await expect(page.locator('#perf-legend')).toContainText('LLC');
  await expect(page.locator('a[href="/legacy"]')).toHaveText(/Exact KUtrace view/);
  await expect(page.locator('#timeline')).toBeVisible();
  const longSpanCoverage = await page.locator('#timeline').evaluate(canvas => {
    const context = canvas.getContext('2d');
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let columns = 0;
    for (let x = 48; x < canvas.width; x++) {
      const offset = (25 * canvas.width + x) * 4;
      const background = data[offset] === 9 && data[offset + 1] === 12 && data[offset + 2] === 18;
      if (!background) columns++;
    }
    return {columns, plotWidth: canvas.width - 48};
  });
  expect(longSpanCoverage.columns).toBeGreaterThan(longSpanCoverage.plotWidth * .9);
  if (browserName === 'chromium') {
    await expect(page).toHaveScreenshot('workspace.png', {
      animations: 'disabled',
      fullPage: true,
      mask: [page.locator('#query-status')],
      maskColor: '#121722'
    });
  }
});

test('composes filters, runs SQL, zooms, and restores saved state', async ({page}) => {
  await page.locator('#filter-field').selectOption('category');
  await page.locator('#filter-op').selectOption('=');
  await page.locator('#filter-value').fill('agent');
  await page.locator('#add-filter').click();
  await expect(page.locator('#filter-chips')).toContainText('category = agent');
  await expect(page.locator('#event-count')).toContainText('4 rows');

  await page.locator('#sql').fill('SELECT COUNT(*) AS agent_count FROM agent_spans');
  await page.locator('#run-sql').click();
  await expect(page.locator('#query-table tbody td').first()).toHaveText('4');
  await expect(page.locator('#query-status')).toContainText('1 rows');

  const initialRange = await page.locator('#range-label').textContent();
  await page.locator('#zoom-in').click();
  await expect(page.locator('#range-label')).not.toHaveText(initialRange);
  await page.keyboard.press('Digit0');
  await expect(page.locator('#range-label')).toHaveText(initialRange);
  await page.keyboard.press('Equal');
  await expect(page.locator('#range-label')).not.toHaveText(initialRange);
  const zoomedRange = await page.locator('#range-label').textContent();
  await page.keyboard.press('ArrowRight');
  await expect(page.locator('#range-label')).not.toHaveText(zoomedRange);
  const pannedRange = await page.locator('#range-label').textContent();
  await page.locator('#sql').focus();
  await page.keyboard.press('ArrowLeft');
  await expect(page.locator('#range-label')).toHaveText(pannedRange);
  await page.locator('#timeline').focus();
  await page.keyboard.press('Home');
  await expect(page.locator('#range-label')).toHaveText(initialRange);

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

test('supports bounded pointer selection and trackpad-style navigation', async ({page}) => {
  const timeline = page.locator('#timeline');
  const box = await timeline.boundingBox();
  expect(box).not.toBeNull();
  const initialRange = await page.locator('#range-label').textContent();
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * .3, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * .7, y, {steps: 5});
  await expect(page.locator('#time-selection')).toBeVisible();
  await page.mouse.up();
  await expect(page.locator('#range-label')).not.toHaveText(initialRange);

  const selectedRange = await page.locator('#range-label').textContent();
  await page.mouse.move(box.x + box.width * .6, y);
  await page.keyboard.down('Control');
  await page.mouse.wheel(0, -180);
  await page.keyboard.up('Control');
  await expect(page.locator('#range-label')).not.toHaveText(selectedRange);

  const zoomedRange = await page.locator('#range-label').textContent();
  await page.keyboard.down('Shift');
  await page.mouse.wheel(0, 120);
  await page.keyboard.up('Shift');
  await expect(page.locator('#range-label')).not.toHaveText(zoomedRange);

  await timeline.dblclick({position: {x: box.width / 2, y: box.height / 2}});
  await expect(page.locator('#range-label')).toHaveText(initialRange);
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
      if(payload.sql.includes('WITH RECURSIVE scoped_events'))timelineQueries.push(payload.sql);
    }
    await route.continue();
  });
  await page.reload();
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
      if(payload.sql==='SELECT MIN(ts),MAX(ts_end) FROM events'){
        await route.fulfill({contentType:'application/json',body:JSON.stringify({columns:['MIN(ts)','MAX(ts_end)'],rows:[[0,10]],truncated:false,elapsed_ms:0.1})});
        return;
      }
      if(wideCpuCount&&payload.sql.startsWith('SELECT DISTINCT cpu FROM events')){
        await route.fulfill({contentType:'application/json',body:JSON.stringify({columns:['cpu'],rows:Array.from({length:wideCpuCount},(_,cpu)=>[cpu]),truncated:false,elapsed_ms:0.1})});
        return;
      }
      if(payload.sql.includes('ranked AS')&&payload.sql.includes('SELECT bucket,cpu,event'))timelineQueries.push(payload.sql);
    }
    await route.continue();
  });
  await page.reload();
  await expect(page.locator('#timeline')).toHaveAttribute('data-ready','true');
  await expect(page.locator('#timeline')).toHaveAttribute('data-source','mipmap');
  await expect(page.locator('#timeline')).toHaveAttribute('data-mipmap-level','fine');
  expect(timelineQueries.at(-1)).toContain('FROM timeline_mipmap');
  expect(timelineQueries.at(-1)).toContain('long_events');

  wideCpuCount=64;
  await page.reload();
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

test('serves the untouched legacy viewer', async ({page}) => {
  const source=await readFile('../../../hello_world_demo_live.html');
  const response=await page.request.get('/legacy');
  expect(response.status()).toBe(200);
  expect(Buffer.compare(await response.body(),source)).toBe(0);
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
