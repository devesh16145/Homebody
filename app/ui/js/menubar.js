// Menu-bar panel, laid out like Control Center: the state, any question, then a tile per app.
// Click a tile to open that app protected. Models and window apps are toggles (running or not).

import { invoke, onEvent } from './api.js';
import { $, esc, on, patch, initPage, errText, fmtCount, secondsLeft } from './util.js';
import { icon, appTile, mark, markMove, markReset, startBlinking, isNew, setAppIcons } from './icons.js';

initPage();

const root = $('#mb');
let status = null, error = '';
let entering = true;             // the panel was just opened: tiles pop in, the cat peeks up
const busy = new Set();
const plural = (n, one, many) => `${fmtCount(n)} ${n === 1 ? one : many}`;

function head(s) {
  let st = 'private', title = 'Private', sub = s.today.blocked ? `${fmtCount(s.today.blocked)} stopped at the door` : 'Everyone stayed home';
  if (s.setupNeeded) [st, title, sub] = ['ask', 'Not set up', 'Turn on to start'];
  else if (s.lockdown) [st, title, sub] = ['shut', 'All blocked', 'Nothing can leave'];
  else if (s.mode === 'off') [st, title, sub] = ['off', 'Off', 'No protection'];
  else if (s.fence !== 'up') [st, title, sub] = ['ask', 'Needs a fix', 'Open Homebody'];
  else if (s.mode === 'watch') [title, sub] = ['Watching', 'All allowed, all written down'];
  else if (s.pending.length) [st, sub] = ['ask', plural(s.pending.length, 'question', 'questions')];
  return `<header class="mb-head">${mark(36, st, st === 'private', markMove('mb', st))}<div class="grow"><b>${esc(title)}</b><small>${esc(sub)}</small></div></header>`;
}

function question(q) {
  const fresh = isNew('ask:' + q.id);
  return `<div class="mb-q${fresh ? ' is-new' : ''}" data-ask="${esc(q.id)}">
    <div class="mb-q-t">${appTile(q.appIcon, q.appName, 28)}<span class="grow"><small>${esc(q.appName)} wants</small><b>${esc(q.label)}</b></span>
      <span class="mb-left" data-left="${esc(q.expiresAt)}" title="Blocked if you do not answer">${secondsLeft(q)} s</span></div>
    <div class="mb-q-a"><button class="cap sm" data-answer="block">Block</button><button class="cap sm" data-answer="once">Once</button><button class="cap sm tide" data-answer="always">Always</button></div></div>`;
}

function appTileBtn(a) {
  const b = busy.has(a.id);
  const blocked = a.problem || !a.canStart;
  const toggle = a.kind !== 'agent';
  const on = toggle && a.running;
  const what = blocked ? '' : a.kind === 'agent' ? 'open' : a.running ? 'stop' : 'start';
  const word = blocked ? 'Can’t open' : b ? '…' : a.kind === 'agent' ? 'Open' : a.running ? 'Running' : a.kind === 'model' ? 'Start' : 'Open';
  const label = blocked ? `${a.name}: ${a.problem || 'protection for it is not in place yet'}` : what === 'stop' ? `${a.name} is running. Stop it` : `${word} ${a.name}`;
  return `<button class="mb-tile${on ? ' on' : ''}" data-do="${what}" data-id="${esc(a.id)}" aria-label="${esc(label)}" title="${esc(label)}"${toggle ? ` aria-pressed="${on}"` : ''}${blocked || b ? ' disabled' : ''}>
    ${appTile(a.icon, a.name, 44)}<b>${esc(a.name)}</b><small>${esc(word)}</small></button>`;
}

function render() {
  if (!status) return;
  const s = status;
  const apps = s.apps.filter((a) => a.protected && (a.installed || a.custom));
  patch(root, `${head(s)}
    ${error ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(error)}</span></div>` : ''}
    ${s.pending.length ? `<section class="mb-qs">${s.pending.map(question).join('')}</section>` : ''}
    ${s.setupNeeded ? '<button class="cap tide lg" data-do="main">Turn on protection…</button>' : `<section class="mb-tiles" aria-label="Apps">${apps.map(appTileBtn).join('')}</section>`}
    <footer class="mb-foot">
      <button class="cap" data-do="main">Open Homebody</button>
      ${s.setupNeeded ? '' : s.lockdown ? '<button class="cap" data-do="release">Let out</button>' : '<button class="cap danger" data-do="panic" title="Nothing leaves until you let things out again">Block all</button>'}
    </footer>`);
  if (entering) {
    entering = false;
    root.classList.remove('mb-enter'); void root.offsetWidth; root.classList.add('mb-enter');
    // Only this drawing plays the entrance; later redraws must not replay it.
    clearTimeout(enterTimer); enterTimer = setTimeout(() => root.classList.remove('mb-enter'), 800);
  }
}

// Each time the panel opens, play its entrance again (focus and visibility both report it: once).
let enterTimer = 0, openedAt = 0;
function opened() {
  if (document.hidden || Date.now() - openedAt < 500) return;
  openedAt = Date.now(); entering = true; markReset('mb'); root.__html = ''; render();
}
document.addEventListener('visibilitychange', opened);
window.addEventListener('focus', opened);
startBlinking();

async function refresh() { try { status = await invoke('get_status'); render(); loadIcons(); } catch (e) { /* keep last */ } }

// Each app's own icon, asked for again only when the list of apps changes.
let iconsFor = '';
async function loadIcons() {
  const key = status.apps.filter((a) => a.protected).map((a) => a.id).join();
  if (key === iconsFor) return;
  iconsFor = key;
  try {
    const byId = await invoke('get_app_icons');
    const byName = {};
    status.apps.forEach((a) => { if (byId[a.id]) byName[a.name] = byId[a.id]; });
    if (Object.keys(byName).length) { setAppIcons(byName); root.__html = ''; render(); }
  } catch (e) { /* letter tiles stay */ }
}

on(root, 'click', '[data-answer]', async (ev, b) => {
  try { await invoke('decide', { askId: b.closest('[data-ask]').dataset.ask, choice: b.dataset.answer }); } catch (e) { error = errText(e); }
  refresh();
});
on(root, 'click', '[data-do]', async (ev, b) => {
  const what = b.dataset.do, id = b.dataset.id;
  if (!what) return;
  error = '';
  try {
    if (what === 'main') { await invoke('open_main', {}); return; }
    if (what === 'panic') { status = await invoke('set_lockdown', { on: true }); return render(); }
    if (what === 'release') { status = await invoke('set_lockdown', { on: false }); return render(); }
    if (what === 'open') { await invoke('app_open_terminal', { appId: id }); invoke('hide_window').catch(() => {}); return; }
    busy.add(id); render();
    const out = await invoke(what === 'stop' ? 'app_stop' : 'app_start', { appId: id });
    busy.delete(id);
    if (out && out.mode) status = out;
    if (what === 'start') invoke('hide_window').catch(() => {});
    render();
  } catch (e) { busy.delete(id); error = errText(e); render(); }
});
document.addEventListener('keydown', (ev) => { if (ev.key === 'Escape') invoke('hide_window').catch(() => {}); });
setInterval(() => root.querySelectorAll('[data-left]').forEach((n) => { n.textContent = Math.max(0, Math.ceil((new Date(n.dataset.left) - Date.now()) / 1000)) + ' s'; }), 1000);
onEvent((evt) => { if (evt.type === 'status') { status = evt.data; render(); loadIcons(); } else if (evt.type === 'ask' || evt.type === 'ask_resolved') refresh(); });
refresh();
