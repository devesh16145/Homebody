// Tests for the extension without an editor: a stand-in for the `vscode` module, and a
// stand-in for Homebody that speaks its real line protocol over a real socket.
//   node test/run.js            everything
//   node test/run.js --live     also talk to the Homebody running on this computer (read-only)
'use strict';
const assert = require('assert');
const net = require('net');
const os = require('os');
const path = require('path');
const fs = require('fs');
const Module = require('module');

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'hb-vsc-'));
process.env.HOMEBODY_DATA_DIR = dir;
const hb = require('../lib/client');

// ---- a stand-in Homebody ----
const world = { lockdown: false, decided: [], trusted: [], subscribers: [], imported: [] };
const statusOf = () => ({ mode: 'private', indicator: world.pending ? 'asking' : 'private', sentence: 'Private. Nothing has tried to leave today.', lockdown: world.lockdown, setupNeeded: false,
  pending: world.pending ? [world.pending] : [],
  apps: [{ id: 'claude-code', name: 'Claude Code', kind: 'agent', installed: true, protected: true, canStart: true, command: 'claude', workspace: '/ws/claude-code', problem: null },
         { id: 'ollama', name: 'Ollama', kind: 'model', installed: true, protected: true, canStart: true, command: 'ollama', workspace: null, problem: null }] });
const commands = {
  get_status: () => statusOf(),
  set_lockdown: (a) => { world.lockdown = !!a.on; return statusOf(); },
  decide: (a) => { world.decided.push(a); world.pending = null; return null; },
  get_trust: () => [{ appId: 'claude-code', host: 'api.anthropic.com', enabled: true }],
  set_trust: (a) => { world.trusted.push(a); return []; },
  get_projects: () => world.imported,
  project_import: (a) => { const p = { name: 'proj', source: a.path, copy: '/ws/claude-code/proj', copyExists: true, suggested: [] }; world.imported.push(p); return p; },
  project_changes: () => ({ total: 2, conflicts: 0, items: [{ kind: 'modified', path: 'a.txt' }, { kind: 'added', path: 'b.txt' }] }),
  project_apply: () => ({ applied: 2, skipped: [] }),
  get_feed: () => [{ ts: new Date().toISOString(), verb: 'blocked', appName: 'Claude Code', label: 'example.org', host: 'example.org', explanation: 'not trusted' }],
  boom: () => { throw 'That is not allowed.'; },
};
const server = net.createServer((sock) => {
  let buf = '';
  sock.on('data', (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const req = JSON.parse(buf.slice(0, i)); buf = buf.slice(i + 1);
      if (req.cmd === 'subscribe') { world.subscribers.push(sock); continue; }
      try {
        if (!commands[req.cmd]) throw `Unknown command: ${req.cmd}`;
        sock.write(JSON.stringify({ ok: true, result: commands[req.cmd](req.args || {}) }) + '\n');
      } catch (e) { sock.write(JSON.stringify({ ok: false, error: String(e) }) + '\n'); }
    }
  });
  sock.on('error', () => {});
});
const emit = (type, data) => world.subscribers.forEach((s) => s.write(JSON.stringify({ event: { type, data } }) + '\n'));

// ---- a stand-in editor ----
const ui = { messages: [], answers: [], picks: [], terminals: [], commands: {}, bar: null, output: [] };
const vscode = {
  StatusBarAlignment: { Right: 2 }, ThemeColor: function (id) { this.id = id; },
  workspace: { workspaceFolders: [{ uri: { fsPath: path.join(dir, 'my-project') } }], getConfiguration: () => ({ get: (k, d) => d }) },
  window: {
    createStatusBarItem: () => (ui.bar = { text: '', tooltip: '', show() {}, dispose() {} }),
    createOutputChannel: () => ({ clear() { ui.output = []; }, appendLine(l) { ui.output.push(l); }, show() {}, dispose() {} }),
    showWarningMessage: async (m, ...buttons) => { ui.messages.push(m); return ui.answers.shift(); },
    showInformationMessage: async (m, ...rest) => { ui.messages.push(m); return ui.answers.shift(); },
    showErrorMessage: async (m) => { ui.messages.push('ERROR ' + m); },
    showQuickPick: async (items, opts) => { const want = ui.picks.shift(); return typeof want === 'function' ? want(items) : want; },
    createTerminal: (o) => { ui.terminals.push(o); return { show() {} }; },
  },
  commands: { registerCommand: (name, fn) => { ui.commands[name] = fn; return { dispose() {} }; }, executeCommand: (name) => ui.commands[name]() },
};
const load = Module._load;
Module._load = (request, ...rest) => (request === 'vscode' ? vscode : load(request, ...rest));

const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, what) => { for (let i = 0; i < 100; i++) { if (fn()) return; await wait(20); } throw new Error('never happened: ' + what); };
let passed = 0;
const test = async (name, fn) => { await fn(); passed++; console.log('  ✓ ' + name); };

(async () => {
  await new Promise((r) => server.listen(hb.socketPath(), r));
  fs.mkdirSync(path.join(dir, 'my-project'));

  console.log('The client');
  await test('a command gets its answer', async () => assert.equal((await hb.call('get_status')).mode, 'private'));
  await test('a refusal comes back as the sentence Homebody gave', async () => { await assert.rejects(hb.call('boom'), /That is not allowed\./); });
  await test('when Homebody is not running, it says so', async () => { await assert.rejects(hb.call('get_status', null, path.join(dir, 'nothing.sock')), /not running/); });
  await test('place patterns match the way the engine matches them', async () => {
    assert.ok(hb.matches('*.example.com', 'a.b.example.com') && hb.matches('example.com', 'EXAMPLE.com'));
    assert.ok(!hb.matches('*.example.com', 'example.com') && !hb.matches('*.example.com', 'evilexample.com'));
  });
  await test('a project file gives only usable places', async () => {
    assert.deepEqual(hb.placesIn('{"trust": ["Registry.NPMJS.org", "pypi.org", "pypi.org", "*", "10.0.0.1", "*.com", 42, "*.example.com", "a b.com"]}'), ['registry.npmjs.org', 'pypi.org', '*.example.com']);
    assert.deepEqual(hb.placesIn('not json'), []);
  });
  await test('the status bar says the one thing that matters', async () => {
    assert.equal(hb.summary(null).text, 'Homebody is not running');
    assert.equal(hb.summary({ indicator: 'private', pending: [] }).text, 'Private');
    assert.equal(hb.summary({ indicator: 'asking', pending: [{}, {}] }).text, '2 questions');
    assert.equal(hb.summary({ indicator: 'private', lockdown: true, pending: [] }).text, 'Everything blocked');
    assert.equal(hb.summary({ indicator: 'watching', pending: [] }).text, 'Watching');
  });

  console.log('The extension');
  const ext = require('../extension');
  const context = { subscriptions: [] };
  ext.activate(context);
  await test('the status bar follows Homebody', async () => { await until(() => ui.bar.text === '$(shield) Private', 'status painted'); assert.match(ui.bar.tooltip, /Nothing has tried to leave/); });
  await test('a question is shown and the answer goes back', async () => {
    await until(() => world.subscribers.length === 1, 'subscribed');
    world.pending = { id: 'q1', appId: 'claude-code', title: 'Claude Code wants to reach example.org.', body: 'Allow it?' };
    ui.answers.push('Always allow');
    emit('ask', world.pending);
    await until(() => world.decided.length === 1, 'answered');
    assert.deepEqual(world.decided[0], { askId: 'q1', choice: 'always' });
    assert.match(ui.messages[0], /wants to reach example\.org/);
  });
  await test('a question closed without choosing is not answered for you', async () => {
    ui.answers.push(undefined);
    emit('ask', { id: 'q2', title: 't', body: 'b' });
    await wait(120);
    assert.equal(world.decided.length, 1);
  });
  await test('an agent opens protected on a copy of the open folder, after you agree', async () => {
    ui.picks.push((items) => items.find((i) => i.app.id === 'claude-code'));
    ui.answers.push('Bring this folder in');
    await ui.commands['homebody.openAgent']();
    assert.equal(world.imported[0].source, path.join(dir, 'my-project'));
    const t = ui.terminals[0];
    assert.equal(t.name, 'Claude Code (protected)');
    assert.deepEqual(t.shellArgs, ['run', 'claude-code', '--cwd', '/ws/claude-code/proj', '--', 'claude']);
  });
  await test('declining to bring the folder in opens nothing', async () => {
    world.imported.length = 0;
    ui.picks.push((items) => items[0]);
    ui.answers.push(undefined);
    await ui.commands['homebody.openAgent']();
    assert.equal(ui.terminals.length, 1);
  });
  await test('only agents are offered, not models', async () => {
    let offered;
    ui.picks.push((items) => { offered = items.map((i) => i.label); return undefined; });
    await ui.commands['homebody.openAgent']();
    assert.deepEqual(offered, ['Claude Code']);
  });
  await test('changes are listed and put back only when you choose to', async () => {
    world.imported.push({ name: 'proj', source: path.join(dir, 'my-project'), copy: '/ws/claude-code/proj', copyExists: true });
    ui.messages.length = 0;
    ui.picks.push((items) => { assert.equal(items.length, 3); return items[0]; });
    await ui.commands['homebody.reviewChanges']();
    assert.match(ui.messages[ui.messages.length - 1], /2 file\(s\) put into this folder/);
  });
  await test('places a project asks for are trusted only when ticked', async () => {
    fs.writeFileSync(path.join(dir, 'my-project', '.homebody.json'), '{"trust": ["registry.npmjs.org", "api.anthropic.com", "pypi.org"]}');
    ui.picks.push((items) => items[0]);                       // the agent
    ui.picks.push((items) => { assert.deepEqual(items.map((i) => i.label), ['registry.npmjs.org', 'pypi.org']); return [items[1]]; });
    await ui.commands['homebody.trustProjectPlaces']();
    assert.deepEqual(world.trusted, [{ appId: 'claude-code', host: 'pypi.org', enabled: true }]);
  });
  await test('block everything, and release', async () => {
    await ui.commands['homebody.blockEverything']();
    assert.equal(ui.bar.text, '$(circle-slash) Everything blocked');
    await ui.commands['homebody.release']();
    assert.equal(ui.bar.text, '$(shield) Private');
  });
  await test('recent activity is written out', async () => { await ui.commands['homebody.showActivity'](); assert.match(ui.output[0], /Blocked\s+Claude Code\s+example\.org/); });
  await test('when Homebody goes away the bar says so', async () => {
    world.subscribers.forEach((s) => s.destroy());
    await new Promise((r) => server.close(r));
    await until(() => ui.bar.text.includes('not running'), 'noticed');
  });
  context.subscriptions.forEach((d) => d.dispose && d.dispose());

  if (process.argv.includes('--live')) {
    console.log('The Homebody running on this computer');
    delete process.env.HOMEBODY_DATA_DIR;
    await test('answers with its status', async () => { const s = await hb.call('get_status', null, hb.socketPath()); assert.ok(Array.isArray(s.apps) && typeof s.sentence === 'string'); console.log('      ' + s.sentence); });
    await test('every agent says how it is started', async () => { const s = await hb.call('get_status', null, hb.socketPath()); assert.ok(s.apps.filter((a) => a.kind === 'agent').every((a) => typeof a.command === 'string' && a.command)); });
  }
  console.log(`\n${passed} passed`);
  fs.rmSync(dir, { recursive: true, force: true });
  process.exit(0);
})().catch((e) => { console.error('\nFAILED: ' + (e.stack || e)); process.exit(1); });
