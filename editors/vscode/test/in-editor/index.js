// The extension, run inside a real editor (VS Code, Cursor) against the Homebody running on
// this computer. Started by test/in-editor/run.sh, which gives the editor a throw-away profile
// and a throw-away folder; this file is loaded by the editor itself, so `vscode` is the real thing.
//
// Pop-ups are answered by this script (it stands in for the person), everything else is real:
// the commands, the status bar, the terminal, the engine, the fence. It writes what it found
// to $HOMEBODY_TEST_OUT as JSON. It undoes what it did: the project it brought in is removed
// and "block everything" is released.
'use strict';
const vscode = require('vscode');
const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const hb = require('../../lib/client');

const AGENT = process.env.HOMEBODY_TEST_AGENT || 'claude-code';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(fn, ms = 10000) {
  const end = Date.now() + ms;
  for (;;) {
    let v = null;
    try { v = await fn(); } catch (e) { v = null; }
    if (v) return v;
    if (Date.now() > end) return null;
    await sleep(200);
  }
}

exports.run = async function run() {
  const results = [];
  const check = (name, ok, detail = '') => results.push({ name, ok: !!ok, detail: String(detail == null ? '' : detail).slice(0, 400) });
  const save = () => fs.writeFileSync(process.env.HOMEBODY_TEST_OUT, JSON.stringify({ editor: `${vscode.env.appName} ${vscode.version}`, results }, null, 1));
  // The launcher opens the folder by a path that goes through a link (/tmp on a Mac), on purpose.
  const opened = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0].uri.fsPath;
  const dir = fs.realpathSync(opened);
  const cli = hb.findCli('');
  let project = null;

  // The person's answers. Each pop-up is recorded, then answered by the current `answer`.
  const real = { pick: vscode.window.showQuickPick, info: vscode.window.showInformationMessage, warn: vscode.window.showWarningMessage, error: vscode.window.showErrorMessage, terminal: vscode.window.createTerminal };
  const seen = { pick: [], info: [], warn: [], error: [], terminal: [] };
  const answer = { pick: () => undefined, info: () => undefined, warn: () => undefined };
  vscode.window.showQuickPick = async (items, opts) => { items = await items; seen.pick.push({ items, opts }); return answer.pick(items, opts || {}); };
  vscode.window.showInformationMessage = async (...a) => { seen.info.push(a); return answer.info(...a); };
  vscode.window.showWarningMessage = async (...a) => { seen.warn.push(a); return answer.warn(...a); };
  vscode.window.showErrorMessage = async (...a) => { seen.error.push(a); return undefined; };
  vscode.window.createTerminal = (o) => { seen.terminal.push(o); return real.terminal.call(vscode.window, o); };
  const errorsSince = (n) => seen.error.slice(n).map((a) => a[0]).join(' | ');
  const run = async (command) => { const n = seen.error.length; await vscode.commands.executeCommand(command); return errorsSince(n); };

  try {
    check('the pop-ups can be answered by this script', vscode.window.showQuickPick !== real.pick);

    // 1. It loads and starts.
    const ext = vscode.extensions.getExtension('homebody.homebody');
    check('the editor has the extension', ext);
    const api = await ext.activate();
    check('it starts without an error', ext.isActive && api && typeof api.state === 'function');
    const commands = (await vscode.commands.getCommands(true)).filter((c) => c.startsWith('homebody.')).sort();
    const declared = ext.packageJSON.contributes.commands.map((c) => c.command).sort();
    check('every command it declares is there', JSON.stringify(commands) === JSON.stringify(declared), commands.join(' '));

    // 2. The status bar says what Homebody says.
    const up = await until(() => api.state().running, 8000);
    const status = await hb.call('get_status');
    const want = hb.summary(status);
    check('it finds the running Homebody', up, api.state().text);
    check('the status bar shows Homebody\'s state', api.state().text === `$(${want.icon}) ${want.text}` && api.state().tooltip === status.sentence, `${api.state().text} / ${api.state().tooltip}`);

    // 3. The menu.
    let err = await run('homebody.menu');
    const menu = seen.pick[seen.pick.length - 1];
    check('the menu offers its five things', !err && menu && menu.items.length === 5 && menu.items.some((i) => /Block everything now/.test(i.label)), err || (menu && menu.items.map((i) => i.label).join(' | ')));

    // 4. Activity.
    err = await run('homebody.showActivity');
    const feed = await hb.call('get_feed', { limit: 60 });
    check('recent activity is listed', !err && api.state().activityLines === feed.length && feed.length > 0, err || `${api.state().activityLines} lines, engine has ${feed.length}`);

    // 5. Open an agent protected in this folder. The folder carries a settings file that names
    //    another program as "the homebody command"; a project must not be able to choose it.
    const app = status.apps.find((a) => a.id === AGENT);
    check('the agent is ready in Homebody', app && app.canStart, app && (app.problem || ''));
    const named = JSON.parse(fs.readFileSync(path.join(dir, '.vscode', 'settings.json'), 'utf8'))['homebody.cliPath'];
    const setting = vscode.workspace.getConfiguration('homebody');
    check('the folder\'s settings file names another program as the homebody command', named && fs.existsSync(named), named);
    check('the editor does not take that setting from a folder', setting.get('cliPath') === '' && !setting.inspect('cliPath').workspaceValue, JSON.stringify(setting.inspect('cliPath')));
    answer.pick = (items) => items.find((i) => i.app && i.app.id === AGENT);
    answer.info = (text, opts, ...buttons) => (buttons.includes('Bring this folder in') ? 'Bring this folder in' : undefined);
    err = await run('homebody.openAgent');
    const asked = seen.info.find((a) => /works on a copy of this folder/.test(a[0]));
    check('it explains the copy and asks before bringing the folder in', asked && asked[1] && asked[1].modal === true, err);
    const list = await hb.call('get_projects', { appId: AGENT });
    project = list.find((p) => p.source === dir) || null;
    check('the folder was brought in as a copy', project && project.copyExists && fs.existsSync(path.join(project.copy, 'hello.txt')), project ? project.copy : err);
    const term = seen.terminal[0];
    check('a terminal was opened for the agent', !err && term && /\(protected\)$/.test(term.name), err || (term && term.name));
    check('the program it runs is Homebody\'s own, not the one the folder named', term && term.shellPath === cli && term.shellPath !== named, term && term.shellPath);
    check('the agent is run protected, in the copy', term && project && JSON.stringify(term.shellArgs) === JSON.stringify(['run', AGENT, '--cwd', project.copy, '--', app.command]), term && JSON.stringify(term.shellArgs));
    const live = vscode.window.terminals.find((t) => term && t.name === term.name);
    const pid = live ? await until(() => live.processId, 8000) : null;
    check('the terminal\'s program started', pid, pid);
    // The agent's own process belongs to the private user Homebody made for it, not to you.
    const fenced = () => cp.execSync('ps -axo user=,comm=', { encoding: 'utf8' }).split('\n').filter((l) => /^_hb\S*\s/.test(l) && l.includes(app.command)).map((l) => l.trim().replace(/\s+/g, ' '));
    const inside = await until(() => { const f = fenced(); return f.length ? f : null; }, 15000);
    check('the agent runs as its own private user, not as you', inside && live && live.exitStatus === undefined, inside ? inside[0] : `none found; exit ${live && JSON.stringify(live.exitStatus)}`);
    if (live) live.dispose();
    const closedAt = Date.now();
    const stopped = await until(() => fenced().length === 0, 45000);
    const took = Math.round((Date.now() - closedAt) / 1000);
    check('closing the terminal stops the agent', inside && stopped && took <= 10, stopped ? `it took ${took} s` : `still running after ${took} s: ${fenced().join(' | ')}`);

    // 6. Places the project asks for: shown, never trusted because a file says so.
    const before = await hb.call('get_trust', { appId: AGENT });
    let offered = null;
    answer.pick = (items, opts) => { if (opts.canPickMany) { offered = items.map((i) => i.label); return []; } return items.find((i) => i.app && i.app.id === AGENT); };
    err = await run('homebody.trustProjectPlaces');
    const after = await hb.call('get_trust', { appId: AGENT });
    check('only the undecided places are offered', !err && JSON.stringify(offered) === JSON.stringify(['homebody-editor-test.example']), err || JSON.stringify(offered));
    check('nothing is trusted unless ticked', JSON.stringify(before.map((t) => [t.host, t.enabled])) === JSON.stringify(after.map((t) => [t.host, t.enabled])));

    // 7. The agent changes a file in its copy (run as the agent, inside the fence); review it and take it.
    if (project) {
      const wrote = cp.spawnSync(cli, ['run', AGENT, '--cwd', project.copy, '--', '/bin/sh', '-c', 'echo written by the agent > from-agent.txt'], { encoding: 'utf8', timeout: 30000 });
      check('the agent can write in its copy', wrote.status === 0 && fs.existsSync(path.join(project.copy, 'from-agent.txt')), wrote.stderr);
      check('and not in your folder', !fs.existsSync(path.join(dir, 'from-agent.txt')));
      let shownChanges = null;
      answer.pick = (items) => { shownChanges = items.map((i) => i.label); return items.find((i) => i.apply); };
      const n = seen.info.length;
      err = await run('homebody.reviewChanges');
      check('the change is shown for review', !err && shownChanges && shownChanges.some((l) => /from-agent\.txt/.test(l)), err || JSON.stringify(shownChanges));
      const told = seen.info.slice(n).map((a) => a[0]).join(' | ');
      check('taking it puts the file in your folder', fs.existsSync(path.join(dir, 'from-agent.txt')) && /1 file\(s\) put into this folder/.test(told), told);
    }

    // 8. A question from Homebody, answered here.
    const place = 'example.net';
    if (status.ask === false || before.some((t) => hb.matches(t.host, place))) {
      check('a question appears here and the answer counts', false, 'not run: asking is off, or there is already a rule for ' + place);
    } else {
      answer.warn = (text, ...buttons) => (buttons.includes('Allow once') ? 'Allow once' : undefined);
      const n = seen.warn.length;
      const curl = cp.spawn(cli, ['run', AGENT, '--', '/usr/bin/curl', '-sS', '-m', '40', '-o', '/dev/null', '-w', '%{http_code}', `https://${place}/`]);
      let outText = '';
      curl.stdout.on('data', (d) => { outText += d; });
      const ended = new Promise((r) => curl.on('close', r));
      const q = await until(() => seen.warn.slice(n).find((a) => a[0].includes(place)), 15000);
      check('a question appears here', q, q ? q[0] : 'no question arrived');
      await Promise.race([ended, sleep(45000)]);
      check('"Allow once" here lets that one connection through', outText.trim() === '200', outText.trim());
      const rules = await hb.call('get_trust', { appId: AGENT });
      check('and leaves no lasting rule', !rules.some((t) => hb.matches(t.host, place)));
      const cleared = await until(async () => ((await hb.call('get_status')).pending || []).length === 0, 6000);
      check('no question is left waiting', cleared, JSON.stringify(((await hb.call('get_status')).pending || []).map((p) => p.host || p.title)));
    }

    // 9. Block everything, and release.
    err = await run('homebody.blockEverything');
    const blocked = (await hb.call('get_status')).lockdown === true;
    check('"Block everything now" blocks everything', !err && blocked && /Everything blocked/.test(api.state().text), err || api.state().text);
    err = await run('homebody.release');
    const now = await hb.call('get_status'), nowWant = hb.summary(now);
    check('and releasing lifts it', !err && now.lockdown === false && api.state().text === `$(${nowWant.icon}) ${nowWant.text}` && !/Everything blocked/.test(api.state().text), err || api.state().text);

    // 10. The project file is checked as you type.
    const bad = vscode.Uri.file(path.join(dir, 'bad', '.homebody.json'));
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(bad));
    const marks = await until(() => { const d = vscode.languages.getDiagnostics(bad); return d.length ? d : null; }, 15000);
    check('a wrong .homebody.json is marked in the editor', marks, marks ? marks.map((d) => d.message).join(' | ') : 'no marks appeared');

    check('no error message was shown along the way', seen.error.length === 0, errorsSince(0));
  } catch (e) {
    check('the test ran to the end', false, (e && e.stack) || e);
  } finally {
    try { await hb.call('set_lockdown', { on: false }); } catch (e) { /* not running */ }
    if (project) { try { await hb.call('project_remove', { appId: AGENT, name: project.name }); } catch (e) { check('the test project was removed', false, e.message); } }
    save();
  }
  const failed = results.filter((r) => !r.ok);
  if (failed.length) throw new Error(`${failed.length} of ${results.length} failed`);
};
