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
    } else if (action.startsWith('dock:')) {
      const dock = action.slice('dock:'.length);
      await page.locator(`[data-dock="${dock}"]`).click();
    } else if (action.startsWith('agent:')) {
      const span = action.slice('agent:'.length);
      await page.locator('[data-dock="agent"]').click();
      await page.locator(`[data-agent-span="${span}"]`).click();
    } else if (action.startsWith('search:')) {
      await page.locator('#trace-search').fill(action.slice('search:'.length));
    } else {
      throw new Error(`unknown action: ${action}`);
    }
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
    highlightedTracks: (await timeline.getAttribute('data-highlighted-tracks') ?? '')
      .split(',')
      .filter(Boolean),
    renderedEvents: Number(await timeline.getAttribute('data-rendered-events')),
    selection: (await page.locator('#selection-summary').textContent())?.trim() ?? '',
    agentContext: (await page.locator('#agent-context-title').textContent())?.trim() ?? '',
    searchMatches: (await page.locator('#search-count').textContent())?.trim() ?? '',
  };
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
} finally {
  await browser.close();
}
