// Feed: one live, chronological list. Rows are keyed by event id and patched in place.
import { invoke } from './api.js';
import {
  $, esc, on, patch, errText, isToday, feedTime, fullTime, fmtBytes, fmtDuration, fmtCount, verbChip, trustKey,
} from './util.js';
import { icon, appTile } from './icons.js';

const PAGE = 200;
let root, ctx, bar, list;
let loaded = false, loading = false, visible = false, loadError = '', hasMore = false;
const byId = new Map();     // id -> FeedEvent
let order = [];             // ids, newest first
let filter = 'all';         // all | today | blocked | app:<id>
let openId = null;
const trust = new Map();    // app|host -> TrustEntry
const undo = new Map();     // app|host -> 'on' | 'off' | 'none' (state before the last change)
const rowError = new Map(); // event id -> message

export function init(el, context) {
  root = el; ctx = context;
  on(root, 'click', '.chip[data-filter]', (ev, b) => { filter = b.dataset.filter; openId = null; renderBar(); renderList(); });
  on(root, 'click', '.ev-row', (ev, b) => toggle(Number(b.closest('.ev').dataset.id)));
  on(root, 'click', '[data-do]', (ev, b) => act(b.dataset.do, b));
  setInterval(() => { if (visible) root.querySelectorAll('.ev-time').forEach((t) => { t.textContent = feedTime(t.dateTime); }); }, 30000);
}

function build() {
  if (root.querySelector('.feed')) return;
  root.__html = null;
  root.innerHTML = '<div class="feed"><div class="feed-bar" role="toolbar" aria-label="Filters"></div><div class="feed-list scroll"></div></div>';
  bar = $('.feed-bar', root); list = $('.feed-list', root);
}

export function show() {
  visible = true;
  build();
  renderBar();
  if (!loaded) load(); else { renderList(); refreshTrust(); }
}
export function hide() { visible = false; }

export function onStatus() {
  if (!visible) return;
  renderBar();
  refreshOpen();
}

export function onEvent({ type, data }) {
  if (type === 'ask_resolved') refreshTrust();
  if (type !== 'feed' || !loaded) return;
  const isNew = !byId.has(data.id);
  byId.set(data.id, data);
  if (isNew) { order.push(data.id); order.sort((a, b) => b - a); }
  if (!visible || !list) return;
  const li = list.querySelector(`.ev[data-id="${data.id}"]`);
  if (li) return patch(li, liInner(data));
  if (!isNew || !matches(data)) return;
  const ul = list.querySelector('ul');
  if (!ul || order[0] !== data.id) return renderList();
  ul.insertAdjacentHTML('afterbegin', liHtml(data, true));
}

// ---------- data ----------

async function load() {
  if (loading) return;
  loading = true; loadError = '';
  renderList();
  try {
    const [events, entries] = await Promise.all([invoke('get_feed', { limit: PAGE }), invoke('get_trust', {})]);
    events.forEach((e) => { if (!byId.has(e.id)) byId.set(e.id, e); });
    order = [...byId.keys()].sort((a, b) => b - a);
    hasMore = events.length >= PAGE;
    setTrust(entries);
    loaded = true;
  } catch (e) { loadError = errText(e); }
  loading = false;
  renderList();
}

async function loadMore() {
  try {
    const more = await invoke('get_feed', { limit: PAGE, beforeId: order[order.length - 1] });
    more.forEach((e) => byId.set(e.id, e));
    order = [...byId.keys()].sort((a, b) => b - a);
    hasMore = more.length >= PAGE;
  } catch (e) { ctx.toast(errText(e)); }
  renderList();
}

function setTrust(entries, appId) {
  if (!Array.isArray(entries)) return;
  const apps = new Set(entries.map((t) => t.appId));
  if (appId) apps.add(appId);
  [...trust.values()].forEach((t) => { if (!appId || apps.has(t.appId)) trust.delete(trustKey(t.appId, t.host)); });
  entries.forEach((t) => trust.set(trustKey(t.appId, t.host), t));
}
async function refreshTrust() {
  try { setTrust(await invoke('get_trust', {})); refreshOpen(); } catch (e) { /* keep what we have */ }
}
function trustFor(e) {
  const exact = trust.get(trustKey(e.appId, e.host));
  if (exact) return exact;
  for (const t of trust.values()) {
    if (t.appId === e.appId && t.host.startsWith('*.') && e.host.endsWith(t.host.slice(1))) return t;
  }
  return null;
}

// ---------- render ----------

function matches(e) {
  if (filter === 'today') return isToday(e.ts);
  if (filter === 'blocked') return e.verb === 'blocked';
  if (filter.startsWith('app:')) return e.appId === filter.slice(4);
  return true;
}

function renderBar() {
  const s = ctx.status();
  if (!bar || !s) return;
  const chip = (id, label, extra = '') => `<button class="chip" data-filter="${esc(id)}" data-key="f-${esc(id)}" aria-pressed="${filter === id}">${extra}${esc(label)}</button>`;
  const apps = s.apps.filter((a) => a.installed).map((a) => chip('app:' + a.id, a.name, appTile(a.icon, a.name, 16))).join('');
  patch(bar, chip('all', 'All') + chip('today', 'Today') + chip('blocked', 'Blocked') + (apps ? '<span class="sep"></span>' + apps : '')
    + `<span class="feed-sum num">Today: ${fmtCount(s.today.allowed)} allowed · ${fmtCount(s.today.blocked)} blocked</span>`);
}

function bytesCell(e) {
  if (e.open && e.verb !== 'asked') return '<span class="live">Live</span>';
  if ((e.verb === 'allowed' || e.verb === 'seen') && e.bytesOut != null) {
    return `<span title="Sent ${esc(fmtBytes(e.bytesOut))}${e.bytesIn != null ? ', received ' + esc(fmtBytes(e.bytesIn)) : ''}">${esc(fmtBytes(e.bytesOut))}<small>sent</small></span>`;
  }
  return '';
}

function rowHtml(e, open) {
  return `<button class="ev-row" data-key="row" aria-expanded="${open}">
    ${verbChip(e.verb)}
    <span class="ev-app">${appTile(e.appIcon, e.appName)}<span class="ellipsis">${esc(e.appName)}</span></span>
    <span class="ev-what ellipsis">${esc(e.label || e.host)}${e.purpose ? `<span class="ev-purpose">${esc(e.purpose)}</span>` : ''}</span>
    <span class="ev-bytes num">${bytesCell(e)}</span>
    <time class="ev-time num" datetime="${esc(e.ts)}">${esc(feedTime(e.ts))}</time></button>`;
}

function actionsHtml(e) {
  const err = rowError.get(e.id);
  const errHtml = err ? `<span class="notice error" role="alert">${esc(err)}</span>` : '';
  const btn = (act, label, cls = '') => `<button class="btn sm ${cls}" data-do="${act}" data-key="${act}">${label}</button>`;
  const ask = e.verb === 'asked' ? ctx.status().pending.find((a) => a.appId === e.appId && a.host === e.host) : null;
  if (ask) {
    return `<span class="ev-state">Waiting for your answer</span><span data-ask="${esc(ask.id)}" class="row">`
      + btn('once', 'Allow once') + btn('always', 'Always allow') + btn('block', 'Block') + '</span>' + errHtml;
  }
  if (e.reason === 'around_gate') {
    return '<span class="ev-state">Refused by the fence. Protected apps can only reach the network through the gate.</span>';
  }
  if (e.reason === 'malformed' && !/^\d|:/.test(e.host || '') && e.label !== e.host) {
    return '<span class="ev-state">Stopped by Sandbox-ai. The place itself is still trusted.</span>';
  }
  if (e.reason === 'malformed' || e.reason === 'local_address') {
    return '<span class="ev-state">This kind of address is always blocked.</span>';
  }
  const t = trustFor(e), k = trustKey(e.appId, e.host);
  let html = '';
  if (t && t.enabled) html = `<span class="ev-state on">${icon('check')}Trusted for ${esc(e.appName)}</span>` + btn('trust-off', 'Block');
  else if (t) html = `<span class="ev-state off">${icon('x')}Blocked for ${esc(e.appName)}</span>` + btn('trust-on', 'Always allow');
  else html = btn('trust-on', 'Always allow') + btn('trust-off', 'Block');
  if (undo.has(k)) html += '<button class="link" data-do="undo" data-key="undo">Undo</button>';
  return html + errHtml;
}

function detailHtml(e) {
  const where = e.host ? `${e.host}:${e.port}` : e.port ? `port ${e.port}, address not reported by macOS` : '';
  const meta = [where ? `<span class="mono sel">${esc(where)}</span>` : '', `<span>${esc(fullTime(e.ts))}</span>`];
  if (e.bytesOut != null) meta.push(`<span class="num">Sent ${esc(fmtBytes(e.bytesOut))}${e.bytesIn != null ? ' · received ' + esc(fmtBytes(e.bytesIn)) : ''}${e.durationMs != null ? ' · ' + esc(fmtDuration(e.durationMs)) : ''}</span>`);
  const inside = e.requests && e.requests.length
    ? `<div class="ev-inside"><span class="ev-inside-h">Inside this connection</span><ul>${e.requests.map((r) => `<li><span class="mono sel">${esc(r.what)}</span>${r.times > 1 ? `<span class="num">${esc(String(r.times))} times</span>` : ''}</li>`).join('')}</ul></div>`
    : e.requests ? '<div class="ev-inside"><span class="ev-inside-h">Looked inside: no requests were made in this connection</span></div>' : '';
  return `<div class="ev-detail"><p class="sel">${esc(e.explanation)}</p>${inside}
    <div class="ev-meta">${meta.join('')}</div>
    <div class="ev-actions">${actionsHtml(e)}</div></div>`;
}

const liInner = (e) => rowHtml(e, e.id === openId) + (e.id === openId ? detailHtml(e) : '');
function liHtml(e, enter = false) {
  return `<li class="ev${enter ? ' enter' : ''}" data-id="${e.id}"${e.id === openId ? ' data-open' : ''}>${liInner(e)}</li>`;
}

function renderList() {
  if (!list) return;
  if (loading && !loaded) {
    list.innerHTML = '<div class="feed-skel" aria-busy="true">' + '<div><span class="skel"></span><span class="skel"></span><span class="skel" style="width:55%"></span><span class="skel"></span></div>'.repeat(7) + '</div>';
    return;
  }
  if (loadError) {
    list.innerHTML = `<div class="empty"><div class="empty-art">${icon('alert')}</div><h2>The Feed could not be loaded</h2><p>${esc(loadError)}</p><button class="btn" data-do="reload">Try again</button></div>`;
    return;
  }
  const rows = order.map((id) => byId.get(id)).filter(matches);
  if (!rows.length) { list.innerHTML = emptyHtml(); return; }
  list.innerHTML = `<ul>${rows.map((e) => liHtml(e)).join('')}</ul>`
    + (hasMore ? '<div class="feed-more"><button class="btn" data-do="more">Show earlier</button></div>' : '');
}

function emptyHtml() {
  if (!order.length) {
    return `<div class="empty"><div class="empty-art">${icon('feed')}</div><h2>Nothing yet</h2>
      <p>When a protected app contacts anything, it appears here with what happened and why. Start a protected app to see it.</p>
      <button class="btn" data-do="apps">Go to Apps</button></div>`;
  }
  const app = filter.startsWith('app:') ? ctx.status().apps.find((a) => a.id === filter.slice(4)) : null;
  const text = filter === 'blocked' ? 'Nothing has been blocked.' : filter === 'today' ? 'Nothing has happened today.'
    : `${app ? app.name : 'This app'} has not contacted anything.`;
  return `<div class="empty"><h2>Nothing here</h2><p>${esc(text)}</p><button class="btn" data-do="all">Show everything</button></div>`;
}

function refreshLi(id) {
  const e = byId.get(id), li = list && list.querySelector(`.ev[data-id="${id}"]`);
  if (!e || !li) return;
  li.toggleAttribute('data-open', id === openId);
  patch(li, liInner(e));
}
const refreshOpen = () => { if (openId != null && visible) refreshLi(openId); };

function toggle(id) {
  const prev = openId;
  openId = prev === id ? null : id;
  rowError.delete(id);
  if (prev != null) refreshLi(prev);
  refreshLi(id);
}

// ---------- actions ----------

async function act(what, btn) {
  if (what === 'reload') return load();
  if (what === 'more') return loadMore();
  if (what === 'apps') return ctx.go('studio', 'apps');
  if (what === 'all') { filter = 'all'; renderBar(); return renderList(); }
  const li = btn.closest('.ev');
  const e = li && byId.get(Number(li.dataset.id));
  if (!e) return;
  rowError.delete(e.id);
  if (what === 'once' || what === 'always' || what === 'block') {
    try { await invoke('decide', { askId: btn.parentElement.dataset.ask, choice: what }); }
    catch (err) { rowError.set(e.id, errText(err)); refreshLi(e.id); }
    return;
  }
  const k = trustKey(e.appId, e.host), before = trust.get(k);
  const restore = () => { if (before) trust.set(k, before); else trust.delete(k); };
  let call;
  if (what === 'undo') {
    const prev = undo.get(k);
    undo.delete(k);
    if (prev === 'none') { trust.delete(k); call = invoke('remove_trust', { appId: e.appId, host: e.host }); }
    else { trust.set(k, { ...before, enabled: prev === 'on' }); call = invoke('set_trust', { appId: e.appId, host: e.host, enabled: prev === 'on' }); }
  } else {
    const enabled = what === 'trust-on';
    undo.set(k, before ? (before.enabled ? 'on' : 'off') : 'none');
    trust.set(k, { ...(before || { appId: e.appId, host: e.host, source: 'you' }), enabled });
    call = invoke('set_trust', { appId: e.appId, host: e.host, enabled });
  }
  refreshLi(e.id);                       // instant visual result
  try { setTrust(await call, e.appId); }
  catch (err) { restore(); undo.delete(k); rowError.set(e.id, errText(err)); }
  refreshLi(e.id);
}
