// Proof: the hero screen. The streak, the statement, and an honest account of its limits.
import { invoke } from './api.js';
import { $, esc, on, patch, errText, fmtBytes, fmtCount, feedTime, dot } from './util.js';
import { icon } from './icons.js';

let root, ctx, el = null;
let visible = false, data = null, loadError = '';
let verify = { state: 'idle', error: '' };   // idle | running | failed
let share = { state: 'idle', path: '', error: '' }; // idle | saving | saved | failed
let week = null;                               // the last seven days in numbers
let stamp = { state: 'idle', text: '', error: '' };   // idle | running | done | failed
let poll = 0, soon = 0;

export function init(node, context) {
  root = node; ctx = context;
  on(root, 'click', '[data-do]', (ev, b) => actions[b.dataset.do] && actions[b.dataset.do]());
}

function build() {
  if (el && root.contains(el.hero)) return;
  root.__html = null;
  root.innerHTML = `<div class="proof scroll"><div class="proof-in">
    <div id="pf-hero"></div><div id="pf-stats"></div><div id="pf-chain"></div><div id="pf-week"></div><div id="pf-honest"></div></div></div>`;
  el = { inner: $('.proof-in', root), hero: $('#pf-hero', root), stats: $('#pf-stats', root), chain: $('#pf-chain', root), week: $('#pf-week', root), honest: $('#pf-honest', root) };
}

export function show() {
  visible = true;
  build();
  render();
  load();
  clearInterval(poll);
  poll = setInterval(load, 60000);     // keeps the streak text current
}
export function hide() { visible = false; clearInterval(poll); }
export function onStatus() { loadSoon(300); }
export function onEvent({ type }) { if (type === 'feed') loadSoon(900); }

function loadSoon(ms) {
  if (!visible) return;
  clearTimeout(soon);
  soon = setTimeout(load, ms);
}

async function load() {
  try { data = await invoke('get_proof'); loadError = ''; }
  catch (e) { if (!data) loadError = errText(e); }
  try { week = await invoke('get_summary', { days: 7 }); } catch (e) { /* the rest of Proof still stands */ }
  if (visible) render();
}

// ---------- render ----------

function render() {
  if (!data) {
    patch(el.hero, loadError
      ? `<div class="empty"><div class="empty-art">${icon('alert')}</div><h2>Proof could not be loaded</h2><p>${esc(loadError)}</p><button class="btn" data-do="reload">Try again</button></div>`
      : '<div class="proof-skel" aria-busy="true"><span class="skel" style="width:90px"></span><span class="skel" style="width:340px;height:34px;border-radius:10px"></span><span class="skel" style="width:420px"></span><span class="skel" style="width:300px"></span></div>');
    return;
  }
  const priv = data.mode === 'private' && !!data.privateSince;
  el.inner.classList.toggle('plain', !priv);
  patch(el.hero, priv ? heroHtml() : idleHtml());
  patch(el.stats, statsHtml());
  patch(el.chain, chainHtml());
  patch(el.week, weekHtml());
  patch(el.honest, `<div class="honest">
    <div><h3>What this proof covers</h3><p class="sel">${esc(data.covers)}</p></div>
    <div><h3>What it cannot see</h3><p class="sel">${esc(data.cannotSee)}</p></div></div>
    <p class="proof-key">Signed by this computer. Key <span class="mono sel">${esc(data.publicKey)}</span></p>`);
}

function heroHtml() {
  const fresh = /^just now$/i.test(data.streakText);
  const leaked = data.untrustedBytesSincePrivate;
  return `<div class="proof-over">${dot('private')}Proof</div>
    <h1 class="streak">${fresh ? 'Private since just now' : 'Private for ' + esc(data.streakText)}</h1>
    <p class="statement sel">${esc(data.statement)}</p>
    ${leaked ? `<div class="zero warn">${icon('info')}<span class="num">${esc(fmtBytes(leaked))} went to places you allowed once</span></div>`
      : `<div class="zero good">${icon('check')}<span class="num">0 bytes to anywhere else</span></div>`}`;
}

function idleHtml() {
  const why = data.mode === 'watch'
    ? 'Watch is on, so everything is allowed and written down. There is no streak to show until Private is on.'
    : 'Protection is off, so your AI apps can reach anything. There is no streak to show until Private is on.';
  return `<div class="proof-over">Proof</div>
    <h1 class="streak">Not private right now</h1>
    <p class="statement">${why}</p>
    <div class="proof-cta"><button class="btn primary lg" data-do="private" data-key="private">Turn on Private</button></div>`;
}

function statsHtml() {
  const n = data.appsProtected;
  return `<div class="stats">
    <div class="stat"><b class="num">${esc(fmtBytes(data.bytesOutToday))}</b><span>sent to trusted places today</span></div>
    <div class="stat"><b class="num">${fmtCount(data.blockedTotal)}</b><span>${data.blockedTotal === 1 ? 'attempt' : 'attempts'} blocked</span></div>
    <div class="stat" title="${esc(data.appNames.join(', '))}"><b class="num">${fmtCount(n)}</b><span>${n === 1 ? 'app' : 'apps'} protected</span></div></div>`;
}

function weekHtml() {
  if (!week || !week.apps.length) return '';
  const plural = (n, one, many) => `${fmtCount(n)} ${n === 1 ? one : many}`;
  return `<div class="week"><h3>The last seven days</h3>${week.apps.map((a) => {
    const stopped = [a.aroundGate ? `${plural(a.aroundGate, 'attempt', 'attempts')} to go around the gate` : '', a.secretsStopped ? `${plural(a.secretsStopped, 'request', 'requests')} carrying a secret` : '', a.addressesRefused ? `${plural(a.addressesRefused, 'request', 'requests')} to an address that is switched off` : ''].filter(Boolean);
    return `<div class="week-app"><div class="week-h"><b>${esc(a.appName)}</b><span class="num">${plural(a.allowed, 'connection', 'connections')} allowed · ${fmtCount(a.blocked)} blocked · ${esc(fmtBytes(a.bytesOut))} sent</span></div>
      ${a.places.length ? `<div class="week-l"><span>Reached</span><span class="sel">${a.places.map((p) => `${esc(p.name)} <i class="num">${esc(fmtBytes(p.bytesOut))}</i>`).join(', ')}</span></div>` : ''}
      ${a.blockedPlaces.length ? `<div class="week-l"><span>Blocked</span><span class="sel">${a.blockedPlaces.map((p) => `${esc(p.name)} <i class="num">${fmtCount(p.times)}×</i>`).join(', ')}</span></div>` : ''}
      ${stopped.length ? `<div class="week-l"><span>Stopped</span><span>${stopped.join(', ')}</span></div>` : ''}</div>`;
  }).join('')}</div>`;
}

function chainHtml() {
  const c = data.chain;
  let cls = '', glyph = 'receipt', title = 'The record has not been checked yet', sub = 'Verify confirms nothing in it was changed or removed.';
  if (verify.state === 'running') { cls = 'run'; title = 'Checking every receipt…'; sub = 'This can take a moment.'; }
  else if (verify.state === 'failed') { cls = 'bad'; glyph = 'alert'; title = 'The check could not finish'; sub = esc(verify.error) + ' Try again in a moment.'; }
  else if (c && c.ok) { cls = 'ok'; glyph = 'check'; title = `Chain intact · ${fmtCount(c.count)} ${c.count === 1 ? 'receipt' : 'receipts'}`; sub = 'Checked ' + esc(feedTime(c.checkedAt).toLowerCase()); }
  else if (c) { cls = 'bad'; glyph = 'alert'; title = 'The record does not add up'; sub = esc(c.error || 'Something in it was changed.') + ' Keep a copy from Studio, then check again.'; }
  const busy = verify.state === 'running', saving = share.state === 'saving';
  let note = '';
  if (share.state === 'saved') note = `<div class="notice good proof-note" role="status">${icon('check')}<span>Saved to Downloads <span class="mono sel">${esc(share.path)}</span></span></div>`;
  if (share.state === 'failed') note = `<div class="notice error proof-note" role="alert">${icon('alert')}<span>${esc(share.error)} Nothing was saved. Try again.</span></div>`;
  return `<div class="chain">
      <span class="chain-ic ${cls}">${icon(glyph)}</span>
      <div class="grow"><div class="chain-t num" role="status">${title}</div><div class="chain-s">${sub}</div></div>
      <button class="btn" data-do="verify" data-key="verify"${busy ? ' disabled' : ''}>${busy ? 'Verifying…' : 'Verify'}</button>
      <button class="btn primary" data-do="share" data-key="share"${saving ? ' disabled' : ''}>${icon('share')}${saving ? 'Saving…' : 'Share proof'}</button>
    </div>${note}
    <div class="stamp"><span class="grow"><b>Independent timestamp.</b> A public time-stamping service can vouch for when this record existed. Only a fingerprint of the record is sent to it, nothing from the record itself.${stamp.state === 'done' ? ` <span class="ok">${esc(stamp.text)}</span>` : ''}${stamp.state === 'failed' ? ` <span class="bad">${esc(stamp.error)}</span>` : ''}</span>
      <button class="btn" data-do="stamp" data-key="stamp"${stamp.state === 'running' ? ' disabled' : ''}>${stamp.state === 'running' ? 'Asking…' : 'Add a timestamp'}</button></div>`;
}

// ---------- actions ----------

const actions = {
  reload: load,
  async private() {
    try { ctx.setStatus(await invoke('set_mode', { mode: 'private' })); await load(); }
    catch (e) { ctx.toast(errText(e)); }
  },
  async verify() {
    verify = { state: 'running', error: '' };
    render();
    try { data.chain = await invoke('verify_chain'); verify = { state: 'idle', error: '' }; }
    catch (e) { verify = { state: 'failed', error: errText(e) }; }
    if (visible) render();
  },
  async stamp() {
    stamp = { state: 'running', text: '', error: '' };
    render();
    try { const r = await invoke('timestamp_log'); stamp = { state: 'done', text: `Vouched for: ${fmtCount(r.records)} records, ${r.time}.`, error: '' }; }
    catch (e) { stamp = { state: 'failed', text: '', error: errText(e) }; }
    if (visible) render();
  },
  async share() {
    share = { state: 'saving', path: '', error: '' };
    render();
    try { const res = await invoke('export_proof'); share = { state: 'saved', path: res.path, error: '' }; }
    catch (e) { share = { state: 'failed', path: '', error: errText(e) }; }
    if (visible) render();
  },
};
