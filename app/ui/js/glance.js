// Glance: the menu-bar popover. One sentence, one switch, the last three events.
import { invoke, onEvent } from './api.js';
import { $, esc, on, patch, initPage, errText, relTime, verbChip, dot, drain } from './util.js';
import { icon, appTile } from './icons.js';

initPage();

const el = {
  status: $('#g-status'), alert: $('#g-alert'), main: $('#g-main'),
  apps: $('#g-apps'), recent: $('#g-recent'), foot: $('#g-foot'),
};
let status = null, events = null, error = '';

// ---------- render ----------

function render() {
  if (!status) return renderLoading();
  patch(el.status, `${dot(status.indicator)}<span>${esc(status.sentence)}</span>`);
  renderAlert();
  patch(el.main, status.setupNeeded ? setupHtml() : switchHtml());
  patch(el.apps, appsHtml());
  patch(el.recent, recentHtml());
  patch(el.foot, footHtml());
}

function renderLoading() {
  patch(el.status, '<span class="dot"></span><span class="skel"></span>');
  patch(el.main, '<div class="g-card g-skel"><span class="skel" style="width:40%"></span><span class="skel" style="width:70%"></span></div>');
  patch(el.recent, '<div class="g-skel"><span class="skel" style="width:85%"></span><span class="skel" style="width:70%"></span><span class="skel" style="width:78%"></span></div>');
}

function renderAlert() {
  const ask = status.pending[0];
  let html = '';
  if (status.lockdown) {
    html += `<div class="g-lock" role="alert">${icon('lock')}<div class="grow"><b>Everything is blocked</b>
      <span>Nothing can leave right now.</span></div>
      <button class="btn sm" data-act="release">Release</button></div>`;
  }
  if (ask) {
    const more = status.pending.length - 1;
    html += `<div class="g-card g-ask" role="alertdialog" aria-labelledby="ask-t" aria-describedby="ask-b">
      <div class="g-ask-top">${appTile(ask.appIcon, ask.appName, 28)}
        <div class="grow"><div class="g-ask-title" id="ask-t">${esc(ask.title)}</div>
        <div class="g-ask-body" id="ask-b">${esc(ask.body)}${more > 0 ? `<span class="g-ask-more">+${more} more waiting</span>` : ''}</div></div></div>
      <div class="g-ask-btns" data-ask="${esc(ask.id)}">
        <button class="btn" data-choice="once">Allow once</button>
        <button class="btn" data-choice="always">Always allow</button>
        <button class="btn" data-choice="block">Block</button></div>
      <div class="g-ask-bar"><i></i></div></div>`;
  }
  if (patch(el.alert, html) && ask) drain($('.g-ask-bar i', el.alert), ask);
}

function switchHtml() {
  const priv = status.mode === 'private', watch = status.mode === 'watch';
  const sub = priv ? 'Only places you trust can be reached.'
    : watch ? 'Paused while Watch is on.' : 'Turn on to keep your AI on this computer.';
  return `<div class="g-card">
    <label class="g-private"><div class="grow"><h1>Private</h1><p>${sub}</p></div>
      <span class="switch lg green"><input type="checkbox" role="switch" data-mode="private" data-key="private" aria-label="Private"${priv ? ' checked' : ''}><i></i></span></label>
    <label class="g-watch"><span class="grow">${watch && status.watch
      ? `<b>Watching</b> ${status.watch.until ? `until ${esc(new Date(status.watch.until).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }))}` : 'until you switch it off'}. ${status.watch.places ? `${status.watch.places} new ${status.watch.places === 1 ? 'place' : 'places'} so far.` : 'Nothing new so far.'}`
      : '<b>Watch:</b> allow everything for an hour and show me what my AI contacts'}</span>
      <span class="switch sm"><input type="checkbox" role="switch" data-mode="watch" data-key="watch" aria-label="Watch"${watch ? ' checked' : ''}><i></i></span></label>
  </div>${status.watchReview && !watch ? `<div class="g-card g-review"><span class="grow"><b>Watch finished.</b> ${status.watchReview.places} new ${status.watchReview.places === 1 ? 'place was' : 'places were'} contacted.</span>
      <button class="btn sm primary" data-act="watch-review">Review</button></div>` : ''}${error ? `<div class="notice error g-err" role="alert">${icon('alert')}<span>${esc(error)}</span></div>` : ''}`;
}

function setupHtml() {
  const reason = status.setupReasons[0];
  return `<div class="g-card g-setup">
    <h1>${reason ? 'Protection needs turning back on' : 'Not protected yet'}</h1>
    <p>${esc(reason || 'One quick step and your AI stays on this computer.')}</p>
    <button class="btn primary lg block" data-act="setup">Finish setup</button></div>`;
}

function appState(a) {
  if (!a.installed) return ['', 'not installed'];
  if (!a.protected) return ['', 'not protected'];
  if (a.fence !== 'up') return ['warn', 'needs setup'];
  if (a.kind === 'agent') return ['on', 'protected'];
  return a.running ? ['on', 'protected and running'] : ['idle', 'protected, not running'];
}

function appsHtml() {
  const apps = status.apps.filter((a) => a.installed && a.protected);
  if (!apps.length) return '';
  return `<div class="g-apps"><span class="g-head">Protected</span>${apps.map((a) => {
    const [cls, words] = appState(a);
    return `<span class="g-app" title="${esc(a.name)}: ${words}">${appTile(a.icon, a.name)}<span class="ellipsis">${esc(a.name)}</span><span class="state ${cls}"></span><span class="sr">${words}</span></span>`;
  }).join('')}</div>`;
}

function recentHtml() {
  if (!events) return '';
  const head = '<div class="g-head g-recent-head">Recent</div>';
  if (!events.length) {
    return head + `<p class="g-empty">${status.setupNeeded ? 'Nothing yet.' : 'Nothing yet. What your AI contacts will show up here.'}</p>`;
  }
  return head + events.map((e) => `<div class="g-ev">${verbChip(e.verb)}
    <span class="what ellipsis">${esc(e.appName)} <span>· ${esc(e.label || e.host)}</span></span>
    <time datetime="${esc(e.ts)}" class="num">${relTime(e.ts)}</time></div>`).join('');
}

function footHtml() {
  return `<div class="g-foot-row">
      <button class="btn" data-open="feed">Feed</button>
      <button class="btn" data-open="proof">Proof</button>
      <button class="iconbtn" data-open="studio" aria-label="Studio" title="Studio">${icon('gear')}</button></div>
    ${status.lockdown || status.setupNeeded ? '' : '<div class="g-panic"><button class="link" data-act="panic">Block everything</button></div>'}`;
}

// ---------- actions ----------

async function run(fn) {
  error = '';
  try { const out = await fn(); if (out && out.mode) status = out; } catch (e) { error = errText(e); }
  render();
}

on(document, 'change', 'input[data-mode]', (ev, input) => {
  const mode = input.checked ? input.dataset.mode : 'off';
  const before = status;
  status = { ...status, mode };            // instant visual result
  render();
  run(async () => {
    try { return await invoke('set_mode', { mode }); } catch (e) { status = before; throw e; }
  });
});

on(document, 'click', '[data-choice]', (ev, btn) => {
  const askId = btn.parentElement.dataset.ask;
  status = { ...status, pending: status.pending.filter((a) => a.id !== askId) };
  render();
  run(() => invoke('decide', { askId, choice: btn.dataset.choice }));
});

on(document, 'click', '[data-open]', (ev, btn) => { invoke('open_main', { tab: btn.dataset.open }).catch(() => {}); });

on(document, 'click', '[data-act]', (ev, btn) => {
  const act = btn.dataset.act;
  if (act === 'setup') invoke('open_main', {}).catch(() => {});
  if (act === 'watch-review') invoke('open_main', { tab: 'feed' }).catch(() => {});
  if (act === 'panic') run(() => invoke('set_lockdown', { on: true }));
  if (act === 'release') run(() => invoke('set_lockdown', { on: false }));
});

document.addEventListener('keydown', (ev) => {
  if (ev.key === 'Escape') invoke('hide_window').catch(() => {});
});

// ---------- data ----------

function upsertEvent(e) {
  if (!events) return;
  const i = events.findIndex((x) => x.id === e.id);
  if (i >= 0) events[i] = e; else events.unshift(e);
  events.sort((a, b) => b.id - a.id);
  events.length = Math.min(events.length, 3);
}

onEvent(({ type, data }) => {
  if (type === 'status') status = data;
  else if (type === 'feed') upsertEvent(data);
  else if (!status) return;
  else if (type === 'ask') {
    const i = status.pending.findIndex((a) => a.id === data.id);
    if (i >= 0) status.pending[i] = data; else status.pending.push(data);
  } else if (type === 'ask_resolved') status.pending = status.pending.filter((a) => a.id !== data.id);
  render();
});

async function load() {
  render();
  try {
    [status, events] = await Promise.all([invoke('get_status'), invoke('get_feed', { limit: 3 })]);
    render();
  } catch (e) {
    patch(el.status, '<span class="dot d-attention"></span><span>Cannot reach the app right now.</span>');
    patch(el.main, `<div class="notice error">${icon('alert')}<span>${esc(errText(e))}</span><button class="link" data-act="retry">Try again</button></div>`);
    patch(el.recent, '');
  }
}
on(document, 'click', '[data-act="retry"]', load);

setInterval(() => { if (status && events) patch(el.recent, recentHtml()); }, 30000);
load();
