// Homebody in the editor: a status indicator, Homebody's questions, and an agent opened
// protected in the folder you have open. It only ever talks to the Homebody app running on
// this computer, over the app's private socket; it has no network code of its own.
'use strict';
const vscode = require('vscode');
const path = require('path');
const fs = require('fs');
const hb = require('./lib/client');

let bar, feed, status = null, running = false, shown = new Set();

const cfg = () => vscode.workspace.getConfiguration('homebody');
const say = (e) => vscode.window.showErrorMessage(String((e && e.message) || e));
const call = (cmd, args) => hb.call(cmd, args);
const folder = () => { const f = vscode.workspace.workspaceFolders; return f && f.length ? f[0].uri.fsPath : null; };

function paint() {
  const s = hb.summary(running ? status : null);
  bar.text = `$(${s.icon}) ${s.text}`;
  bar.tooltip = s.tip || 'Homebody';
  bar.backgroundColor = s.icon === 'question' || s.icon === 'warning' ? new vscode.ThemeColor('statusBarItem.warningBackground') : undefined;
  bar.show();
}

async function refresh() {
  try { status = await call('get_status'); running = true; } catch (e) { status = null; running = false; }
  paint();
}

/** One of Homebody's questions, shown here. Whoever answers first, here or in Homebody, wins. */
async function ask(q) {
  if (!cfg().get('askInEditor', true) || shown.has(q.id)) return;
  shown.add(q.id);
  const pick = await vscode.window.showWarningMessage(`${q.title} ${q.body}`, 'Allow once', 'Always allow', 'Block');
  const choice = { 'Allow once': 'once', 'Always allow': 'always', 'Block': 'block' }[pick];
  if (!choice) return;
  try { await call('decide', { askId: q.id, choice }); } catch (e) { /* already answered elsewhere */ }
}

function onEvent(ev) {
  if (ev.type === 'status') { status = ev.data; running = true; paint(); (status.pending || []).forEach(ask); }
  if (ev.type === 'ask') ask(ev.data);
  if (ev.type === 'ask_resolved') shown.delete(ev.data && ev.data.id);
}

async function pickAgent() {
  if (!status) throw new Error('Homebody is not running. Open the Homebody app.');
  const agents = status.apps.filter((a) => a.kind === 'agent' && a.installed && a.protected);
  if (!agents.length) throw new Error('No protected agent is installed. Add one in Homebody.');
  const pick = await vscode.window.showQuickPick(agents.map((a) => ({ label: a.name, description: a.canStart ? '' : (a.problem || 'Not ready yet'), app: a })), { placeHolder: 'Which agent?' });
  if (!pick) return null;
  if (!pick.app.canStart) throw new Error(pick.app.problem || `${pick.app.name} is not ready. Open Homebody to see why.`);
  return pick.app;
}

/** The copy of this folder that the agent works on, bringing the folder in if it has not been. */
async function projectFor(app, dir) {
  const list = await call('get_projects', { appId: app.id });
  const have = list.find((p) => p.source === dir);
  if (have && have.copyExists) return have;
  const go = await vscode.window.showInformationMessage(`${app.name} works on a copy of this folder, so it cannot touch anything else of yours. You review its changes before they come back.`, { modal: true }, 'Bring this folder in');
  if (go !== 'Bring this folder in') return null;
  const made = await call('project_import', { appId: app.id, path: dir });
  if (made.suggested && made.suggested.length) vscode.window.showInformationMessage(`This project asks for ${made.suggested.length} place(s). Run "Homebody: Trust places this project asks for" to decide.`);
  return made;
}

async function openAgent() {
  const app = await pickAgent();
  if (!app) return;
  const dir = folder();
  let cwd = app.workspace || undefined;
  if (dir) {
    const project = await projectFor(app, dir);
    if (!project) return;
    cwd = project.copy;
  }
  // Only your own setting counts. A project's settings file must never choose the program
  // that "runs the agent protected": it would simply be run, as you, outside any fence.
  const own = cfg().inspect ? cfg().inspect('cliPath') : null;
  const cli = hb.findCli((own && own.globalValue) || '');
  const args = ['run', app.id].concat(cwd ? ['--cwd', cwd] : [], ['--', app.command]);
  vscode.window.createTerminal({ name: `${app.name} (protected)`, shellPath: cli, shellArgs: args }).show();
}

async function reviewChanges() {
  const dir = folder();
  if (!dir) throw new Error('Open the folder you brought in.');
  if (!status) throw new Error('Homebody is not running. Open the Homebody app.');
  for (const app of status.apps.filter((a) => a.kind === 'agent')) {
    const list = await call('get_projects', { appId: app.id }).catch(() => []);
    const p = list.find((x) => x.source === dir);
    if (!p) continue;
    const set = await call('project_changes', { appId: app.id, name: p.name });
    if (!set.total) return vscode.window.showInformationMessage(`${app.name} has changed nothing in its copy.`);
    const items = set.items.map((c) => ({ label: `${c.kind}  ${c.path}`, description: c.conflict ? 'you changed this too' : '' }));
    const act = await vscode.window.showQuickPick([{ label: `$(check) Put ${set.total} change(s) into this folder`, apply: true }].concat(items), { placeHolder: `${app.name} changed ${set.total} file(s)${set.conflicts ? `, ${set.conflicts} also changed by you` : ''}` });
    if (!act || !act.apply) return;
    const r = await call('project_apply', { appId: app.id, name: p.name, overwrite: false });
    return vscode.window.showInformationMessage(`${r.applied} file(s) put into this folder${r.skipped.length ? `; ${r.skipped.length} left alone because you changed them too` : ''}.`);
  }
  vscode.window.showInformationMessage('This folder has not been brought in for any agent.');
}

async function showActivity() {
  const rows = await call('get_feed', { limit: 60 });
  feed.clear();
  for (const e of rows.slice().reverse()) {
    const verb = { allowed: 'Allowed', blocked: 'Blocked', asked: 'Asked  ', seen: 'Seen   ' }[e.verb] || e.verb;
    feed.appendLine(`${new Date(e.ts).toLocaleTimeString()}  ${verb}  ${e.appName.padEnd(14)} ${e.label}${e.host && e.host !== e.label ? ` (${e.host})` : ''}  — ${e.explanation}`);
  }
  feed.show(true);
}

async function trustProjectPlaces() {
  const dir = folder();
  if (!dir) throw new Error('Open a folder first.');
  let text = '';
  try { text = fs.readFileSync(path.join(dir, '.homebody.json'), 'utf8'); } catch (e) { throw new Error('This folder has no .homebody.json file naming places.'); }
  const wanted = hb.placesIn(text);
  if (!wanted.length) throw new Error('.homebody.json names no places Homebody can use.');
  const app = await pickAgent();
  if (!app) return;
  const trust = await call('get_trust', { appId: app.id });
  const undecided = wanted.filter((h) => !trust.some((t) => t.host === h));
  if (!undecided.length) return vscode.window.showInformationMessage(`Every place this project asks for is already decided for ${app.name}.`);
  const picks = await vscode.window.showQuickPick(undecided.map((h) => ({ label: h })), { canPickMany: true, placeHolder: `Tick the places ${app.name} may reach. Nothing is trusted unless you tick it.` });
  for (const p of picks || []) await call('set_trust', { appId: app.id, host: p.label, enabled: true });
  if (picks && picks.length) vscode.window.showInformationMessage(`${picks.length} place(s) trusted for ${app.name}.`);
}

async function menu() {
  const items = [
    { label: '$(terminal) Open an agent protected, in this folder', run: 'homebody.openAgent' },
    { label: '$(diff) Review what the agent changed', run: 'homebody.reviewChanges' },
    { label: '$(list-unordered) Show recent activity', run: 'homebody.showActivity' },
    { label: '$(checklist) Trust places this project asks for', run: 'homebody.trustProjectPlaces' },
    status && status.lockdown ? { label: '$(play) Stop blocking everything', run: 'homebody.release' } : { label: '$(circle-slash) Block everything now', run: 'homebody.blockEverything' },
  ];
  const pick = await vscode.window.showQuickPick(items, { placeHolder: hb.summary(running ? status : null).tip || 'Homebody' });
  if (pick) vscode.commands.executeCommand(pick.run);
}

function activate(context) {
  bar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 90);
  bar.command = 'homebody.menu';
  feed = vscode.window.createOutputChannel('Homebody');
  const cmd = (name, fn) => context.subscriptions.push(vscode.commands.registerCommand(name, () => Promise.resolve().then(fn).catch(say)));
  cmd('homebody.menu', menu);
  cmd('homebody.openAgent', openAgent);
  cmd('homebody.reviewChanges', reviewChanges);
  cmd('homebody.showActivity', showActivity);
  cmd('homebody.trustProjectPlaces', trustProjectPlaces);
  cmd('homebody.blockEverything', async () => { status = await call('set_lockdown', { on: true }); paint(); });
  cmd('homebody.release', async () => { status = await call('set_lockdown', { on: false }); paint(); });
  const sub = hb.subscribe(onEvent, (up) => { running = up; if (up) refresh(); else { status = null; paint(); } });
  context.subscriptions.push(bar, feed, { dispose: () => sub.stop() });
  paint();
  refresh();
}

function deactivate() {}

module.exports = { activate, deactivate };
