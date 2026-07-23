import {defineConfig} from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  outputDir: './test-results',
  timeout: 30_000,
  expect: {timeout: 8_000},
  fullyParallel: false,
  workers: 1,
  snapshotPathTemplate: '{testDir}/{testFilePath}-snapshots/{arg}-{platform}{ext}',
  projects: [
    {name: 'chromium', use: {browserName: 'chromium'}},
    {name: 'firefox', use: {browserName: 'firefox'}},
  ],
  use: {
    baseURL: 'http://127.0.0.1:38126',
    viewport: {width: 1440, height: 1000},
    colorScheme: 'dark',
    locale: 'en-US',
    timezoneId: 'UTC',
    reducedMotion: 'reduce'
  },
  webServer: [
    {
      command: '../../target/release/kutrace-ui tests/fixtures/agent_trace.json --database tests/fixtures/agent.sqlite --legacy-html ../../../hello_world_demo_live.html --listen 127.0.0.1:38126 --rebuild',
      cwd: '.',
      url: 'http://127.0.0.1:38126/',
      reuseExistingServer: false,
      timeout: 30_000
    },
    {
      command: '../../target/release/kutrace-ui tests/fixtures/stack_trace.json --database tests/fixtures/stack.sqlite --listen 127.0.0.1:38127 --rebuild',
      cwd: '.',
      url: 'http://127.0.0.1:38127/',
      reuseExistingServer: false,
      timeout: 30_000
    },
    {
      command: '../../target/release/kutrace-ui tests/fixtures/rpc_trace.json --database tests/fixtures/rpc.sqlite --listen 127.0.0.1:39130 --rebuild',
      cwd: '.',
      url: 'http://127.0.0.1:39130/',
      reuseExistingServer: false,
      timeout: 30_000
    },
    {
      command: '../../target/release/kutrace-ui tests/fixtures/execution_trace.json --database tests/fixtures/execution.sqlite --listen 127.0.0.1:39133 --rebuild',
      cwd: '.',
      url: 'http://127.0.0.1:39133/',
      reuseExistingServer: false,
      timeout: 30_000
    },
    {
      command: '../../target/release/kutrace-ui tests/fixtures/callout_trace.json --database tests/fixtures/callout.sqlite --listen 127.0.0.1:39134 --rebuild',
      cwd: '.',
      url: 'http://127.0.0.1:39134/',
      reuseExistingServer: false,
      timeout: 30_000
    }
  ]
});
