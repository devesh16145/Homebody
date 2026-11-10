// Watch: the bar that says Watch is on (or has finished), and the sheet that turns
// "here is what your AI contacted" into trusted places.

import { invoke } from './api.js';
import { $, esc, on, patch, errText, fmtCount, fmtBytes } from './util.js';
import { icon, appTile } from './icons.js';

let ctx, sheet;
let open = false, review = null, error = '', busy = false;
const picked = new Set();   // appId|host switched on in the sheet

const key = (i) => `${i.appId}|${i.host}`;
const plural = (n, one, many) => `${fmtCount(n)} ${n === 1 ? one : many}`;
const clock = (iso) => new Date(iso).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

/** The bar under the toolbar. Empty when Watch is neither on nor waiting for a review. */
export function barHtml(status) {
  const w = status.watch, r = status.watchReview;
  if (w) {
    const until = w.until ? `until ${esc(clock(w.until))}, then Private comes back on` : 'until you switch it off';
    const seen = w.places ? `${plural(w.places, 'new place', 'new places')} so far.` : 'Nothing new so far.';
    return `<div class="watchbar" role="status">${icon('info')}<span><b>Watching.</b> Everything is allowed and written down ${until}. ${seen}</span>
      ${w.places ? '<button class="btn sm" data-act="watch-review">See them</button>' : ''}
      <button class="btn sm" data-act="watch-stop">Back to Private</button></div>`;
  }
  if (r) {
    return `<div class="watchbar done" role="status">${icon('check')}<span><b>Watch finished.</b> Your AI contacted ${plural(r.places, 'place that is', 'places that are')} not on your Trusted list.</span>
      <button class="btn sm primary" data-act="watch-review">Review</button></div>`;
  }
  return '';
}

function itemHtml(i, active) {
  const k = key(i);
  const facts = `${plural(i.times, 'time', 'times')}${i.bytesOut ? ` · ${esc(fmtBytes(i.bytesOut))} sent` : ''}`;
  return `<label class="ws-item">
    <div class="grow ws-id"><div class="ws-name">${esc(i.label || i.host)}${i.purpose ? `<span class="ws-purpose">${esc(i.purpose)}</span>` : ''}</div>
      <div class="ws-host"><span class="mono sel">${esc(i.host)}</span><span class="num">${facts}</span></div></div>
    ${active ? '' : `<span class="switch sm green"><input type="checkbox" role="switch" data-pick="${esc(k)}" data-key="pick-${esc(k)}" aria-label="Trust ${esc(i.label || i.host)}"${picked.has(k) ? ' checked' : ''}><i></i></span>`}
  </label>`;
}

function render() {
  if (!open) { sheet.hidden = true; patch(sheet, ''); return; }
  sheet.hidden = false;
  let body;
  if (error && !review) body = `<div class="notice error" role="alert">${icon('alert')}<span>${esc(error)}</span></div>`;
  else if (!review) body = '<p class="ws-empty">Loading…</p>';
  else if (!review.items.length) body = '<p class="ws-empty">Nothing new was contacted. Every place your AI reached was already on your Trusted list.</p>';
  else {
    const apps = [...new Set(review.items.map((i) => i.appId))];
    body = apps.map((id) => {
      const items = review.items.filter((i) => i.appId === id);
      return `<div class="ws-app">${appTile(items[0].appIcon, items[0].appName)}<span>${esc(items[0].appName)}</span></div>
        <div class="ws-list">${items.map((i) => itemHtml(i, review.active)).join('')}</div>`;
    }).join('');
  }
  const active = review && review.active, n = picked.size;
  const lead = active
    ? 'Watch is still on. When it ends you can choose which of these to trust.'
    : 'Switch on the places you want to trust. Anything you leave off will ask you the first time it comes up again.';
  const actions = active
    ? '<button class="btn" data-act="watch-close" data-key="ws-close">Close</button><button class="btn primary" data-act="watch-stop" data-key="ws-stop">Back to Private now</button>'
    : `<button class="btn" data-act="watch-close" data-key="ws-close">Later</button>
       <button class="btn primary" data-act="watch-finish" data-key="ws-finish"${busy || !review ? ' disabled' : ''}>${busy ? 'Saving…' : n ? `Trust ${plural(n, 'place', 'places')}` : 'Done, trust none'}</button>`;
  patch(sheet, `<div class="ws-box" role="dialog" aria-modal="true" aria-labelledby="ws-title" tabindex="-1">
    <h2 id="ws-title">${active ? 'What your AI has contacted so far' : 'What your AI contacted while you watched'}</h2>
    <p class="ws-lead">${lead}</p>
    <div class="ws-body scroll">${body}</div>
    ${error && review ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(error)}</span></div>` : ''}
    <div class="ws-foot">${actions}</div></div>`);
}

async function show() {
  open = true; review = null; error = ''; busy = false; picked.clear();
  render();
  const box = $('.ws-box', sheet); if (box) box.focus();
  try { review = await invoke('get_watch_review'); if (!review) open = false; }
  catch (e) { error = errText(e); }
  render();
}

function close() { open = false; render(); }

async function finish() {
  busy = true; error = ''; render();
  const trust = review.items.filter((i) => picked.has(key(i))).map((i) => ({ appId: i.appId, host: i.host }));
  try {
    ctx.setStatus(await invoke('finish_watch_review', { trust }));
    open = false;
    ctx.toast(trust.length ? `${plural(trust.length, 'place', 'places')} added to Trusted.` : 'Nothing was added to Trusted.');
  } catch (e) { error = errText(e); }
  busy = false; render();
}

async function stop() {
  try { ctx.setStatus(await invoke('set_mode', { mode: 'private' })); if (open) show(); }
  catch (e) { ctx.toast(errText(e)); }
}

export function init(c) {
  ctx = c; sheet = $('#sheet');
  on(document, 'click', '[data-act="watch-review"]', show);
  on(document, 'click', '[data-act="watch-close"]', close);
  on(document, 'click', '[data-act="watch-finish"]', finish);
  on(document, 'click', '[data-act="watch-stop"]', stop);
  on(sheet, 'change', 'input[data-pick]', (ev, input) => { if (input.checked) picked.add(input.dataset.pick); else picked.delete(input.dataset.pick); render(); });
  sheet.addEventListener('click', (ev) => { if (ev.target === sheet) close(); });
  document.addEventListener('keydown', (ev) => { if (open && ev.key === 'Escape') close(); });
  if (new URLSearchParams(location.search).get('review') === 'watch') show();
}

/** Watch ended by its timer while the "so far" sheet was open: show the real review instead. */
export function onStatus(status) {
  if (open && review && review.active && !status.watch) show();
}
