// Main window, organised around the user's AI apps.
//
// The sidebar is the whole map: an overview, one entry per protected app, then the record
// (Activity, Report) and Settings. The most frequent action, opening an AI app, is the biggest
// button on every screen that shows an app. Rules and activity are not separate screens: an
// app's page lists the places it reaches, what happened there, and the switch for each.

import { invoke, onEvent } from './api.js';
import { $, $$, esc, on, patch, initPage, errText, dot, fmtBytes, fmtCount, secondsLeft } from './util.js';
import { icon, appTile, mark } from './icons.js';
import * as proof from './proof.js';
import * as feed from './feed.js';
import * as watch from './watch.js';
import * as appPage from './home-app.js';
import { advanced } from './studio-more.js';

initPage();

const params = new URLSearchParams(location.search);
const el = { nav: $('#nav'), state: $('#state'), view: $('#view'), menu: $('#menu'), toast: $('#toast'), activity: $('#mount-activity'), report: $('#mount-report') };
let status = null;
let route = params.get('go') || 'overview';
let toastTimer = 0;
const busy = new Set();          // app ids with a start/stop/open in progress
const appErrors = new Map();     // app id -> message
const answering = new Set();     // ask ids being answered

export const plural = (n, one, many) => `${fmtCount(n)} ${n === 1 ? one : many}`;

const ctx = {
  status: () => status,
  setStatus(s) { if (s && s.mode) { status = s; render(); } },
  go(tab, sub) {
    if (tab === 'studio' && sub === 'apps') return go('add');
    if (tab === 'studio') return go('settings');
    go(tab === 'feed' ? 'activity' : tab === 'proof' ? 'report' : tab);
  },
  toast(msg) {
    el.toast.textContent = msg;
    el.toast.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.toast.classList.remove('show'), 2600);
  },
};

/** Apps the user has: protected, and either installed or added by them. */
export function myApps(s = status) {
  return (s ? s.apps : []).filter((a) => a.protected && (a.installed || a.custom));
}
const findApp = (id) => (status ? status.apps.find((a) => a.id === id) : null);

// ---------- navigation ----------

function go(next) {
  closeMenu();
  if (next === route) return render();
  route = next;
  el.view.scrollTop = 0;
  render();
  el.view.focus({ preventScroll: true });
}

function navHtml() {
  const apps = myApps();
  const pending = (id) => status.pending.filter((p) => p.appId === id).length;
  const item = (to, inner, extra = '') => `<button class="hm-item${route === to ? ' on' : ''}" data-go="${esc(to)}"${route === to ? ' aria-current="page"' : ''}${extra}>${inner}</button>`;
  const appItem = (a) => {
    const n = pending(a.id);
    const right = n ? `<span class="hm-badge" title="${plural(n, 'question', 'questions')} waiting">${n}</span>`
      : a.running ? '<span class="hm-run" title="Running"></span>' : '';
    return item('app:' + a.id, `${appTile(a.icon, a.name, 24)}<span class="grow ellipsis">${esc(a.name)}</span>${right}`);
  };
  return `<div class="hm-col">
    ${item('overview', `${icon('apps')}<span class="grow">Home</span>`)}
    <div class="hm-sec">Apps</div>
    ${apps.map(appItem).join('')}
    ${item('add', `${icon('plus')}<span class="grow">Add app</span>`, ' data-quiet')}
    <div class="hm-spring"></div>
    <div class="hm-foot">
      ${item('activity', `${icon('feed')}<span>Activity</span>`)}
      ${item('report', `${icon('receipt')}<span>Report</span>`)}
      ${item('settings', `${icon('gear')}<span>Settings</span>`)}
    </div>
  </div>`;
}

// ---------- the state button and its menu ----------

function stateWords(s) {
  if (s.setupNeeded) return ['attention', 'Not set up', 'Turn on to start'];
  if (s.lockdown) return ['blocked', 'All blocked', 'Nothing can leave'];
  if (s.mode === 'off') return ['off', 'Off', 'No protection'];
  if (s.fence !== 'up') return ['attention', 'Needs a fix', 'Protection is not in place'];
  if (s.mode === 'watch') return ['watching', 'Watching', 'All allowed, all written down'];
  if (s.pending.length) return ['asking', 'Private', plural(s.pending.length, 'question', 'questions')];
  return ['private', 'Private', s.today.blocked ? `${fmtCount(s.today.blocked)} stopped today` : 'Everyone stayed home'];
}

/** Which drawing of the mark fits a state. */
export const markState = (ind) => ({ blocked: 'shut', off: 'off', asking: 'ask', attention: 'ask' })[ind] || 'private';

function stateHtml() {
  const [ind, title, sub] = stateWords(status);
  return `${mark(30, markState(ind))}<span class="grow"><b>${esc(title)}</b><small>${esc(sub)}</small></span>${icon('chevron', 'hm-down')}`;
}

function menuHtml() {
  const s = status, m = s.lockdown ? 'lockdown' : s.mode;
  const opt = (id, title, sub, extra = '') => `<button role="menuitemradio" aria-checked="${m === id}" data-mode="${id}"${extra}>
    <span class="hm-tick">${m === id ? icon('check') : ''}</span><span class="grow"><b>${title}</b><small>${sub}</small></span></button>`;
  return `${opt('private', 'Private', 'Only what you allow')}
    ${opt('watch', 'Watch for an hour', 'Allow all, then choose what to keep')}
    ${opt('off', 'Off', 'No protection')}
    <hr>
    ${s.lockdown
      ? '<button role="menuitem" data-mode="release"><span class="hm-tick"></span><span class="grow"><b>Let things out again</b><small>Back to how it was</small></span></button>'
      : `<button role="menuitem" data-mode="lockdown" class="danger"><span class="hm-tick">${icon('stop')}</span><span class="grow"><b>Block everything</b><small>Nothing leaves</small></span></button>`}`;
}

function openMenu() {
  if (!status || status.setupNeeded) return;
  patch(el.menu, menuHtml());
  el.menu.hidden = false;
  el.state.setAttribute('aria-expanded', 'true');
  const r = el.state.getBoundingClientRect();
  el.menu.style.left = r.left + 'px';
  el.menu.style.top = r.bottom + 4 + 'px';
  const first = el.menu.querySelector('[aria-checked="true"]') || el.menu.querySelector('button');
  if (first) first.focus();
}
function closeMenu() {
  if (el.menu.hidden) return;
  el.menu.hidden = true;
  el.state.setAttribute('aria-expanded', 'false');
}

async function setMode(m) {
  closeMenu();
  try {
    if (m === 'lockdown') return ctx.setStatus(await invoke('set_lockdown', { on: true }));
    if (m === 'release') return ctx.setStatus(await invoke('set_lockdown', { on: false }));
    if (status.lockdown) await invoke('set_lockdown', { on: false });
    ctx.setStatus(await invoke('set_mode', m === 'watch' ? { mode: m, minutes: 60 } : { mode: m }));
  } catch (e) { ctx.toast(errText(e)); }
}

// ---------- shared pieces ----------

/** A waiting question, answered right where it is shown. */
export function questionHtml(q, withApp = true) {
  const busyNow = answering.has(q.id);
  return `<div class="hm-q" data-ask="${esc(q.id)}" role="group" aria-label="${esc(q.title)}">
    ${withApp ? appTile(q.appIcon, q.appName, 36) : `<span class="hm-q-ic">${icon('info')}</span>`}
    <div class="grow">
      <div class="hm-q-t">${withApp ? `${esc(q.appName)} wants to reach` : 'Wants to reach'}<b>${esc(q.label)}</b></div>
      <div class="hm-q-s mono" title="${esc(q.purpose || '')}">${esc(q.host)}</div>
    </div>
    <div class="hm-q-a">
      <button class="cap sm" data-answer="block" data-key="qb-${esc(q.id)}" title="Not sure? Block it. You can allow it later."${busyNow ? ' disabled' : ''}>Block</button>
      <button class="cap sm" data-answer="once" data-key="qo-${esc(q.id)}"${busyNow ? ' disabled' : ''}>Once</button>
      <button class="cap sm tide" data-answer="always" data-key="qa-${esc(q.id)}"${busyNow ? ' disabled' : ''}>Always</button>
      <span class="hm-q-left" data-left="${esc(q.expiresAt)}" title="Blocked if you do not answer">${secondsLeft(q)} s</span>
    </div></div>`;
}

async function answer(askId, choice) {
  answering.add(askId); render();
  try { await invoke('decide', { askId, choice }); }
  catch (e) { ctx.toast(errText(e)); }
  answering.delete(askId);
  refresh();
}

/** The one big button for an app. Returns [html, words for why it is disabled]. */
export function primaryAction(a, size = '') {
  const b = busy.has(a.id);
  const cls = `cap tide ${size}`.trim();
  const verb = a.kind === 'model' ? 'Start' : 'Open';
  if (a.problem || !a.canStart) return `<button class="${cls}" disabled>${verb}</button>`;
  if (a.kind === 'agent') return `<button class="${cls}" data-app-do="open" data-id="${esc(a.id)}" data-key="open-${esc(a.id)}" aria-label="Open ${esc(a.name)}">Open</button>`;
  if (a.running) return `<button class="cap ${size}" data-app-do="stop" data-id="${esc(a.id)}" data-key="stop-${esc(a.id)}" aria-label="Stop ${esc(a.name)}"${b ? ' disabled' : ''}>${b ? 'Stopping…' : 'Stop'}</button>`;
  return `<button class="${cls}" data-app-do="start" data-id="${esc(a.id)}" data-key="start-${esc(a.id)}" aria-label="${verb} ${esc(a.name)}"${b ? ' disabled' : ''}>${b ? (verb === 'Start' ? 'Starting…' : 'Opening…') : verb}</button>`;
}

/** One line about an app's state, in plain words. */
export function appLine(a) {
  if (a.problem) return ['warn', 'Can’t protect it where it is'];
  if (a.fence !== 'up') return ['warn', 'Not ready yet'];
  if (a.running) return ['run', a.kind === 'model' ? 'Running' : 'Open'];
  return ['', a.kind === 'model' ? 'Stopped' : 'Ready'];
}

async function appDo(what, id) {
  const a = findApp(id);
  if (!a) return;
  appErrors.delete(id);
  try {
    if (what === 'open') { await invoke('app_open_terminal', { appId: id }); ctx.toast(`${a.name} is opening in a protected terminal.`); return; }
    busy.add(id); render();
    const out = await invoke(what === 'stop' ? 'app_stop' : 'app_start', { appId: id });
    busy.delete(id);
    if (out && out.mode) ctx.setStatus(out); else refresh();
  } catch (e) { busy.delete(id); appErrors.set(id, errText(e)); render(); }
}

// ---------- Overview ----------

function hero(s) {
  const [ind, word] = stateWords(s);
  const since = s.privateSince ? new Date(s.privateSince).toLocaleDateString([], { day: 'numeric', month: 'short' }) : null;
  let sub, act = '';
  if (s.setupNeeded) {
    sub = 'One approval with your Mac password turns it on.';
    act = '<button class="cap tide lg" data-do="setup">Turn on protection</button>';
  } else if (s.lockdown) {
    sub = 'Nothing can leave this Mac.';
    act = '<button class="cap lg" data-mode="release">Let things out again</button>';
  } else if (s.mode === 'off') {
    sub = 'Your AI can reach anything.';
    act = '<button class="cap tide lg" data-mode="private">Turn on</button>';
  } else if (s.fence !== 'up') {
    sub = (s.setupReasons && s.setupReasons[0]) || 'Protection is not in place right now.';
    act = '<button class="cap tide lg" data-do="setup">Fix it</button>';
  } else if (s.mode === 'watch') {
    sub = 'Everything is allowed and written down for now.';
  } else {
    sub = since ? `Your AI has stayed home since ${since}.` : 'Your AI stays home.';
  }
  const t = s.today;
  const stats = s.setupNeeded ? '' : `<div class="hm-stats" aria-label="Today">
      <span><b>${fmtCount(t.allowed)}</b> allowed</span><span><b>${fmtCount(t.blocked)}</b> stopped</span><span><b>${fmtBytes(t.bytesOut)}</b> out</span></div>`;
  return `<section class="hm-hero">
    ${mark(64, markState(ind), ind === 'private' || ind === 'asking')}
    <h1>${esc(s.setupNeeded ? 'Not set up yet' : word)}</h1><p>${esc(sub)}</p>${stats}
    ${act ? `<div class="hm-hero-act">${act}</div>` : ''}
  </section>`;
}

function card(a) {
  const [cls, line] = appLine(a);
  const err = appErrors.get(a.id);
  return `<article class="hm-card" data-card="${esc(a.id)}">
    <button class="hm-card-hit" data-go="app:${esc(a.id)}" aria-label="${esc(a.name)}: details"></button>
    ${appTile(a.icon, a.name, 44)}
    <h3>${esc(a.name)}</h3>
    <p class="hm-card-line ${cls}">${esc(line)}</p>
    ${err ? `<p class="hm-card-err" role="alert">${esc(err)}</p>` : ''}
    <div class="hm-card-act">${a.problem ? `<button class="cap" data-go="app:${esc(a.id)}">How to fix</button>` : primaryAction(a)}</div>
  </article>`;
}

function overviewHtml() {
  const s = status;
  if (s.setupNeeded || setup.step === 3) return firstRunHtml(s);
  const apps = myApps();
  const qs = s.pending.length ? `<section class="hm-qs" aria-label="Questions">${s.pending.map((q) => questionHtml(q)).join('')}</section>` : '';
  const bar = watch.barHtml(s);
  const notes = s.presetNotes && s.presetNotes.length ? `<div class="notice" role="status">${icon('info')}<span>${s.presetNotes.map(esc).join(' ')}</span><button class="cap sm" data-do="ack">Got it</button></div>` : '';
  return `<div class="hm-page">
    ${hero(s)}
    ${installHtml()}
    ${bar ? `<div class="hm-watch">${bar}</div>` : ''}
    ${qs}
    <section class="hm-block"><h2>Apps</h2>
      <div class="hm-cards">${apps.map(card).join('')}<button class="hm-card-add" data-go="add">${icon('plus')}Add app</button></div>
    </section>
    ${notes}
  </div>`;
}

// ---------- First run, and repairing protection ----------
//
// First time: 1 choose which apps stay home, 2 one approval with the Mac password, 3 how to start.
// Coming back to repair is one screen: what is wrong, and the button.

const setup = { step: 1, state: 'idle', error: '' };

function firstRunHtml(s) {
  const reason = s.setupNeeded && s.setupReasons && s.setupReasons[0];
  const step = setup.step === 3 ? 3 : reason ? 2 : setup.step;
  const waiting = setup.state === 'waiting';
  const dots = reason ? '' : `<div class="fr-dots" aria-label="Step ${step} of 3">${[1, 2, 3].map((n) => `<i${n === step ? ' class="on"' : ''}></i>`).join('')}</div>`;
  let body;
  if (step === 1) {
    const found = s.apps.filter((a) => a.installed);
    const rows = found.map((a) => `<label class="hm-row">${appTile(a.icon, a.name, 36)}<span class="grow"><b>${esc(a.name)}</b></span>
        <span class="switch"><input type="checkbox" data-protect="${esc(a.id)}" data-key="protect-${esc(a.id)}" aria-label="Keep ${esc(a.name)} home"${a.protected ? ' checked' : ''}><i></i></span></label>`).join('');
    body = `${mark(72, 'private', true)}
      <h1>Which AI apps should stay home?</h1>
      <p class="fr-lead">They keep working. They just can’t send anything out unless you say so.</p>
      ${rows ? `<div class="hm-list fr-list">${rows}</div>` : '<p class="hm-empty">No AI apps found yet. You can add them later.</p>'}
      <button class="cap tide lg" data-do="fr-next" data-key="fr-next">Continue</button>`;
  } else if (step === 2) {
    body = `${mark(72, reason ? 'ask' : 'private', true)}
      <h1>${reason ? 'Protection needs a fix' : 'One password, and it’s on'}</h1>
      <p class="fr-lead">${reason ? esc(reason) : 'Your Mac asks for your password once. Homebody never sees it.'}</p>
      ${reason ? '' : `<div class="hm-explain fr-explain">
        <div><b>Own space</b><p>Each app runs away from your files.</p></div>
        <div><b>Only what you allow</b><p>Everything else stays home.</p></div>
        <div><b>After restarts too</b><p>It stays on. You can remove it all later.</p></div></div>`}
      <button class="cap tide lg" data-do="setup" data-key="run-setup"${waiting ? ' disabled' : ''}>${waiting ? 'Waiting for your password…' : reason ? 'Fix it' : 'Turn on protection'}</button>
      ${setup.state === 'error' ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(setup.error)} Try again whenever you’re ready.</span></div>` : ''}
      <div class="fr-links">${reason || waiting ? '' : '<button class="link" data-do="fr-back">Back</button>'}<button class="link" data-go="technical">What exactly it sets up</button></div>`;
  } else {
    body = `${mark(72, 'private', true)}
      <h1>It’s on. How do you want to start?</h1>
      <p class="fr-lead">You can switch any time from the menu bar.</p>
      <div class="fr-choices">
        <button class="fr-choice" data-do="fr-start" data-mode-to="private" data-key="start-private"><b>Private <span class="tag">Recommended</span></b><small>Only what you allow. Anything new, you’re asked.</small></button>
        <button class="fr-choice" data-do="fr-start" data-mode-to="watch" data-key="start-watch"><b>Watch for an hour</b><small>Everything allowed and written down. Then you choose what to keep.</small></button>
      </div>`;
  }
  return `<div class="hm-page fr">${body}${dots}</div>`;
}

async function runSetup() {
  const firstTime = !(status.setupReasons && status.setupReasons[0]);
  setup.state = 'waiting'; setup.error = ''; render();
  try {
    const res = await invoke('run_setup');
    if (!res || !res.ok) throw (res && res.message) || 'Setup did not finish.';
    setup.state = 'idle'; setup.step = firstTime ? 3 : 1;
    ctx.setStatus(await invoke('get_status'));
  } catch (e) { setup.state = 'error'; setup.error = errText(e); render(); }
}

async function startWith(mode) {
  try { ctx.setStatus(await invoke('set_mode', mode === 'watch' ? { mode, minutes: 60 } : { mode })); } catch (e) { ctx.toast(errText(e)); }
  setup.step = 1; render();
}

// ---------- "not in Applications yet" ----------

let install = null, installBusy = false;
const installLater = () => { try { return localStorage.getItem('sai.install.later') === '1'; } catch (e) { return false; } };
function installHtml() {
  if (!install || !install.canInstall || installLater()) return '';
  return `<div class="hm-set" role="status">${icon('info')}<span class="grow"><b>Move Homebody to Applications</b><small>Keeps it in one place and easy to find.</small></span>
    <button class="cap quiet sm" data-do="install-later">Not now</button><button class="cap tide sm" data-do="install-go"${installBusy ? ' disabled' : ''}>${installBusy ? 'Moving…' : 'Move'}</button></div>`;
}
invoke('get_install').then((i) => { install = i; render(); }).catch(() => {});

// ---------- Add an app ----------

const add = { found: null, error: '', busy: '', name: '', program: '' };

async function loadFound() {
  try { add.found = await invoke('discover_apps'); add.error = ''; } catch (e) { add.error = errText(e); add.found = []; }
  if (route === 'add') render();
}

function addHtml() {
  if (add.found === null) loadFound();
  const known = status.apps.filter((a) => !a.custom && (!a.installed || !a.protected));
  const found = (add.found || []).map((f) => `<li class="hm-row">
      ${appTile('generic', f.name, 28)}
      <div class="grow"><b>${esc(f.name)}</b><small>${esc(f.note)}</small></div>
      ${f.canProtect ? `<button class="cap tide sm" data-add-found="${esc(f.path)}" data-name="${esc(f.name)}"${add.busy === f.path ? ' disabled' : ''}>${add.busy === f.path ? 'Protecting…' : 'Protect'}</button>` : '<span class="tag">Not possible yet</span>'}</li>`).join('');
  return `<div class="hm-page">
    <header class="hm-head"><h1>Add app</h1></header>
    ${add.error ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(add.error)}</span></div>` : ''}
    <section class="hm-block"><h2>Found on this Mac</h2>
      ${add.found === null ? '<p class="muted">Looking…</p>' : found ? `<ul class="hm-list">${found}</ul>` : '<p class="muted">No other AI apps were found in your Applications folders.</p>'}
    </section>
    <section class="hm-block"><h2>Something else</h2>
      <form class="hm-addform">
        <label>Name <input name="name" value="${esc(add.name)}" placeholder="For example, LM Studio" autocomplete="off" data-key="add-name"></label>
        <label>Program <span class="row"><input name="program" value="${esc(add.program)}" placeholder="A command, or an app" autocomplete="off" data-key="add-program" class="grow"><button type="button" class="cap" data-do="pick">Choose…</button></span></label>
        <button class="cap tide" type="submit"${add.busy === 'form' ? ' disabled' : ''}>${add.busy === 'form' ? 'Protecting…' : 'Protect it'}</button>
      </form>
    </section>
    ${known.length ? `<section class="hm-block"><h2>Protected once installed</h2>
      <ul class="hm-list">${known.map((a) => `<li class="hm-row">${appTile(a.icon, a.name, 28)}<div class="grow"><b>${esc(a.name)}</b><small>${esc(a.problem || 'Not installed')}</small></div></li>`).join('')}</ul></section>` : ''}
  </div>`;
}

async function addApp(name, program, key) {
  add.busy = key; add.error = ''; render();
  try {
    const st = await invoke('app_add', { name, program });
    add.busy = ''; add.name = ''; add.program = ''; add.found = null;
    ctx.setStatus(st);
    const made = st.apps.find((a) => a.name.toLowerCase() === String(name).toLowerCase());
    ctx.toast(`${name} is protected.`);
    if (made) go('app:' + made.id);
  } catch (e) { add.busy = ''; add.error = errText(e); render(); }
}

// ---------- Settings ----------

const set = { folders: null, login: null, update: '', checking: false, error: '' };

async function loadSettings() {
  try { set.folders = await invoke('get_private_folders'); } catch (e) { set.folders = { folders: [] }; }
  try { set.login = (await invoke('get_login_item')).on; } catch (e) { set.login = null; }
  if (route === 'settings') render();
}

function settingsHtml() {
  if (set.folders === null) { set.folders = { folders: [] }; loadSettings(); }
  const s = status;
  const fence = s.setupNeeded ? 'Not set up yet.' : s.fence === 'up' ? 'In place. Checked every 30 seconds.' : 'Not in place right now.';
  const sw = (attr, on, label, sub, extra = '') => `<label class="hm-set">
      <span class="grow"><b>${label}</b><small>${sub}</small></span>
      <span class="switch"><input type="checkbox" ${attr}${on ? ' checked' : ''}${extra}><i></i></span></label>`;
  const folders = (set.folders.folders || []).map((f) => `<li class="hm-row">${icon('folder')}<span class="grow mono ellipsis">${esc(f.path.replace(/^\/Users\/[^/]+\//, '~/'))}</span>
      <button class="cap sm" data-folder-remove="${esc(f.path)}">Remove</button></li>`).join('');
  return `<div class="hm-page">
    <header class="hm-head"><h1>Settings</h1></header>
    <section class="hm-block"><h2>Protection</h2>
      <div class="hm-set"><span class="grow"><b>${s.fence === 'up' ? 'On' : 'Needs attention'}</b><small>${esc(fence)}</small></span>
        <button class="cap" data-do="check">Check now</button></div>
      ${sw('data-set="ask"', s.ask !== false, 'Ask me about new places', 'Off: new places are blocked without asking.')}
    </section>
    <section class="hm-block"><h2>Private folders</h2>
      <p class="hm-sub">Text from these folders is never sent.</p>
      ${folders ? `<ul class="hm-list">${folders}</ul>` : ''}
      <button class="cap" data-do="folder-add">${icon('plus')}Add a folder…</button>
    </section>
    <section class="hm-block"><h2>This Mac</h2>
      ${sw('data-set="login"', !!set.login, 'Open Homebody when I log in', 'Keeps protected apps ready.', set.login === null ? ' disabled' : '')}
      <div class="hm-set"><span class="grow"><b>Version ${esc(s.version)}</b><small>${esc(set.update || 'Checks only when you ask. Nothing is downloaded.')}</small></span>
        <button class="cap" data-do="update"${set.checking ? ' disabled' : ''}>${set.checking ? 'Checking…' : 'Check for updates'}</button></div>
    </section>
    <section class="hm-block"><h2>For someone else</h2>
      <div class="hm-set"><span class="grow"><b>Export the record</b><small>A signed folder anyone can check.</small></span>
        <button class="cap" data-do="export">Export…</button></div>
    </section>
    <section class="hm-block">
      <button class="hm-more" data-go="technical">${icon('sliders')}<span class="grow"><b>Technical details</b><small>What setup installs, removing it</small></span>${icon('chevron')}</button>
    </section>
  </div>`;
}

let techNode = null;
function technicalHtml() { return '<div class="hm-page"><header class="hm-head"><button class="link" data-go="settings">‹ Settings</button><h1>Technical details</h1></header><div id="tech"></div></div>'; }

async function settingsDo(what, b) {
  try {
    if (what === 'check') { ctx.setStatus(await invoke('check_fence')); ctx.toast('Checked.'); }
    if (what === 'update') { set.checking = true; render(); try { set.update = (await invoke('check_update')).message; } finally { set.checking = false; render(); } }
    if (what === 'export') { const r = await invoke('export_log', { reveal: true }); ctx.toast('Saved to ' + r.path.replace(/^\/Users\/[^/]+\//, '~/')); }
    if (what === 'folder-add') { const p = await invoke('pick_folder'); if (p) { set.folders = await invoke('add_private_folder', { path: p }); render(); } }
    if (what === 'folder-remove') { set.folders = await invoke('remove_private_folder', { path: b.dataset.folderRemove }); render(); }
  } catch (e) { ctx.toast(errText(e)); }
}

// ---------- render ----------

function render() {
  if (!status) return;
  patch(el.nav, navHtml());
  patch(el.state, stateHtml());
  el.state.disabled = !!status.setupNeeded;
  const r = status.setupNeeded && !['settings', 'technical', 'report', 'activity'].includes(route) ? 'overview' : route;
  const mounted = r === 'activity' ? 'activity' : r === 'report' ? 'report' : null;
  el.view.hidden = !!mounted;
  el.activity.hidden = mounted !== 'activity';
  el.report.hidden = mounted !== 'report';
  if (mounted === 'activity') { feed.show(); feed.onStatus(); } else feed.hide();
  if (mounted === 'report') { proof.show(); } else proof.hide();
  if (mounted) return;
  if (r.startsWith('app:')) {
    const a = findApp(r.slice(4));
    if (!a) { route = 'overview'; return render(); }
    return appPage.render(el.view, a, env);
  }
  appPage.leave();
  const html = r === 'add' ? addHtml() : r === 'settings' ? settingsHtml() : r === 'technical' ? technicalHtml() : overviewHtml();
  patch(el.view, html);
  if (r === 'technical') {
    const node = $('#tech', el.view);
    if (node !== techNode) { techNode = node; advanced.show(); }
  }
}

const env = { ctx, questionHtml, primaryAction, appLine, plural, go, appErrors, render: () => render() };

async function refresh() {
  try { ctx.setStatus(await invoke('get_status')); } catch (e) { /* keep the last state */ }
}

// ---------- events ----------

on(document, 'click', '[data-go]', (ev, b) => go(b.dataset.go));
el.state.addEventListener('click', () => (el.menu.hidden ? openMenu() : closeMenu()));
on(document, 'click', '[data-mode]', (ev, b) => setMode(b.dataset.mode));
on(document, 'click', '[data-answer]', (ev, b) => answer(b.closest('[data-ask]').dataset.ask, b.dataset.answer));
on(document, 'click', '[data-app-do]', (ev, b) => appDo(b.dataset.appDo, b.dataset.id));
on(el.view, 'click', '[data-do]', (ev, b) => {
  const what = b.dataset.do;
  if (what === 'setup') return runSetup();
  if (what === 'fr-next') { setup.step = 2; return render(); }
  if (what === 'fr-back') { setup.step = 1; return render(); }
  if (what === 'fr-start') return startWith(b.dataset.modeTo);
  if (what === 'install-later') { try { localStorage.setItem('sai.install.later', '1'); } catch (e) { /* private mode */ } return render(); }
  if (what === 'install-go') { installBusy = true; render(); return invoke('install_and_restart').catch((e) => { installBusy = false; ctx.toast(errText(e)); render(); }); }
  if (what === 'ack') return invoke('ack_presets').then(ctx.setStatus).catch((e) => ctx.toast(errText(e)));
  if (what === 'pick') return invoke('pick_program').then((p) => { if (p) { add.program = p; if (!add.name) add.name = p.split('/').pop().replace(/\.app$/, ''); render(); } });
  if (route === 'technical' && advanced.act) return advanced.act(what, b);
  if (route.startsWith('app:')) return appPage.act(what, b);
  settingsDo(what, b);
});
on(el.view, 'click', '[data-folder-remove]', (ev, b) => settingsDo('folder-remove', b));
on(el.view, 'click', '[data-add-found]', (ev, b) => addApp(b.dataset.name, b.dataset.addFound, b.dataset.addFound));
on(el.view, 'input', '.hm-addform input', (ev, i) => { add[i.name] = i.value; });
on(el.view, 'submit', '.hm-addform', (ev) => { ev.preventDefault(); addApp(add.name.trim(), add.program.trim(), 'form'); });
on(el.view, 'change', 'input[data-protect]', async (ev, i) => {
  try { ctx.setStatus(await invoke('app_set_protected', { appId: i.dataset.protect, protected: i.checked })); }
  catch (e) { i.checked = !i.checked; ctx.toast(errText(e)); }
});
on(el.view, 'change', 'input[data-set]', async (ev, i) => {
  try {
    if (i.dataset.set === 'ask') ctx.setStatus(await invoke('set_ask', { on: i.checked }));
    if (i.dataset.set === 'login') set.login = (await invoke('set_login_item', { on: i.checked })).on;
  } catch (e) { i.checked = !i.checked; ctx.toast(errText(e)); }
});
document.addEventListener('click', (ev) => { if (!el.menu.hidden && !el.menu.contains(ev.target) && !el.state.contains(ev.target)) closeMenu(); });
document.addEventListener('keydown', (ev) => {
  if (ev.key === 'Escape') closeMenu();
  if (!el.menu.hidden && (ev.key === 'ArrowDown' || ev.key === 'ArrowUp')) {
    const items = $$('button', el.menu), i = items.indexOf(document.activeElement);
    items[(i + (ev.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length].focus();
    ev.preventDefault();
  }
  if ((ev.metaKey || ev.ctrlKey) && ev.key === ',') { ev.preventDefault(); go('settings'); }
});
setInterval(() => $$('[data-left]').forEach((n) => { n.textContent = Math.max(0, Math.ceil((new Date(n.dataset.left) - Date.now()) / 1000)) + ' s'; }), 1000);

onEvent((evt) => {
  if (evt.type === 'status') return ctx.setStatus(evt.data);
  if (evt.type === 'navigate') return ctx.go(evt.data.tab);
  if (evt.type === 'ask' || evt.type === 'ask_resolved') refresh();
  feed.onEvent(evt);
  proof.onEvent(evt);
  appPage.onEvent(evt);
});

feed.init(el.activity, ctx);
proof.init(el.report, ctx);
watch.init(ctx);
advanced.init({ ctx, isShown: (n) => n === 'advanced' && route === 'technical', body: () => $('#tech', el.view) || document.createElement('div') });
refresh();
