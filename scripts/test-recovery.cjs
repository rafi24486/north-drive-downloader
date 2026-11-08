const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');

test('blocked summary exposes Resume and keeps partial/completed progress', async () => {
  const elements = new Map(), listeners = new Map(), calls = [];
  const element = selector => {
    if (!elements.has(selector)) elements.set(selector, {
      value: '', innerHTML: '', textContent: '', style: {},
      classList: { add() {}, remove() {}, toggle() {} }, setAttribute() {},
      addEventListener(name, handler) { this[name] = handler; },
    });
    return elements.get(selector);
  };
  const plan = { folderName: 'Footage', totalBytes: 200, warnings: [], files: [
    { id: 'one', name: 'one.mp4', relativePath: 'one.mp4', size: 100 },
    { id: 'two', name: 'two.mp4', relativePath: 'two.mp4', size: 100 },
  ] };
  const context = {
    document: { querySelector: element, querySelectorAll: () => [] },
    window: { __TAURI__: {
      core: { invoke: async (command, args) => {
        calls.push({ command, args });
        if (command === 'get_preferences') return { apiKey: 'test', maxParallel: 1, saveDirectory: '/downloads' };
        if (command === 'scan_folder') return plan;
      }},
      event: { listen: (name, handler) => { listeners.set(name, handler); } },
    }},
    setTimeout: () => 1, clearTimeout() {}, setInterval: () => 1, clearInterval() {},
  };
  vm.runInNewContext(fs.readFileSync(require('node:path').join(__dirname, '../ui/app.js'), 'utf8'), context);
  await new Promise(setImmediate);
  element('#linkInput').value = 'https://drive.google.com/drive/folders/testFolder';
  await element('#analyzeBtn').click();
  await element('#startProject').click();
  listeners.get('north:progress')({ payload: { id: 'one', bytes: 100, status: 'completed' } });
  listeners.get('north:progress')({ payload: { id: 'two', bytes: 50, status: 'blocked', message: 'Google blocked this transfer' } });
  listeners.get('north:summary')({ payload: { completed: 1, failed: 1, blocked: true, cancelled: false, errors: [] } });
  assert.match(element('#activeProject').innerHTML, /Stopped by Google/);
  assert.match(element('#activeProject').innerHTML, />.*Resume</);
  assert.doesNotMatch(element('#activeProject').innerHTML, /disabled/);
  assert.match(element('#activeProject').innerHTML, /150 B of 200 B/);
  assert.match(element('#errorSummary').innerHTML, /Resuming cannot override a Google block/);
  await element('#startProject').click();
  assert.equal(calls.filter(c => c.command === 'start_download').length, 2);
  assert.equal(calls.at(-1).args.directory, '/downloads');
  assert.match(element('#activeProject').innerHTML, /150 B of 200 B/);
  assert.match(element('#completedRows').innerHTML, /one.mp4/);
});
