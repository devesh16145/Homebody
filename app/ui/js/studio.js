// Studio: everything beyond the everyday surfaces. Sidebar shell plus Trusted and Apps.
// Logs and Advanced live in studio-more.js.
import { invoke } from './api.js';
import { $, $$, esc, on, patch, errText, fmtBytes, fmtCount, cleanHost } from './util.js';
import { icon, appTile } from './icons.js';
import { logs, advanced } from './studio-more.js';

const PANES = [['trusted', 'Trusted', 'shield'], ['apps', 'Apps', 'apps'], ['logs', 'Logs', 'receipt'], ['advanced', 'Advanced', 'sliders']];
let root, ctx, body;
let visible = false, pane = 'trusted';

// ---------- Trusted ----------

const trusted = {
  entries: null, error: '', apps: '',
  show() { this.apps = ''; this.render(); this.load(); },
  async load() {
    try { this.entries = await invoke('get_trust', {}); this.error = ''; }
    catch (e) { this.error = errText(e); }
    if (visible && pane === 'trusted') this.render();
  },
  onStatus() { this.render(); },
  onEvent({ type }) { if (type === 'ask_resolved') this.load(); },

  render() {
    const apps = ctx.status().apps.filter((a) => a.installed);
    const sig = apps.map((a) => a.id).join(',');
    if (this.apps !== sig || !body.querySelector('.tr')) {
      // Build the frame once so the "add" fields keep what the user typed.
      this.apps = sig;
      body.__html = null;
      body.innerHTML = `<div class="tr"><h2>Trusted</h2>
        <p class="lead">Each switch lets one app reach one place. Everything else is blocked or asks you first.</p>
        ${apps.map((a) => `<section class="tr-app" data-app="${esc(a.id)}">
          <h3>${appTile(a.icon, a.name)}${esc(a.name)}</h3>
          <ul class="tr-list"></ul>
          <form class="tr-add" novalidate>
            <input type="text" class="field sel" name="host" placeholder="Add a destination, like example.com" aria-label="Add a destination for ${esc(a.name)}" spellcheck="false" autocomplete="off" autocapitalize="off">
            <button class="btn" type="submit">Add</button>
            <span class="tr-msg" role="alert"></span>
          </form></section>`).join('') || '<p class="muted">No AI apps were found on this computer yet.</p>'}</div>`;
    }
    $$('.tr-app', body).forEach((sec) => patch($('.tr-list', sec), this.listHtml(sec.dataset.app)));
  },

  listHtml(appId) {
    if (this.error && !this.entries) return `<li class="notice error">${icon('alert')}<span>${esc(this.error)}</span><button class="link" data-do="tr-reload">Try again</button></li>`;
    if (!this.entries) return '<li class="tr-row"><span class="skel" style="width:28px;height:17px;border-radius:9px"></span><span class="skel" style="width:40%"></span></li>'.repeat(2);
    const rows = this.entries.filter((t) => t.appId === appId);
    const app = ctx.status().apps.find((x) => x.id === appId);
    const appName = app ? app.name : appId;
    if (!rows.length) return '<li class="tr-none">Nothing is trusted for this app yet, so it cannot reach anything.</li>';
    return rows.map((t) => {
      const named = t.label && t.label !== t.host;
      return `<li class="tr-row">
        <span class="switch sm green"><input type="checkbox" role="switch" data-trust="${esc(t.host)}" data-key="t-${esc(t.host)}" aria-label="${esc((named ? t.label : t.host) + ' for ' + appName)}"${t.enabled ? ' checked' : ''}><i></i></span>
        <div class="grow"><div class="tr-label ellipsis">${esc(named ? t.label : t.host)}${t.recommended ? ' <span class="tag">Recommended</span>' : ''}</div>
          <div class="tr-sub ellipsis">${esc(t.purpose || (t.source === 'you' ? 'Added by you' : ''))}</div></div>
        <span class="tr-host mono sel">${esc(t.host)}</span>
        ${t.source === 'you' ? `<button class="iconbtn" data-do="tr-remove" data-host="${esc(t.host)}" data-key="rm-${esc(t.host)}" aria-label="Remove ${esc(t.host)}" title="Remove">${icon('x')}</button>` : '<span class="tr-gap"></span>'}</li>`;
    }).join('');
  },

  take(list, appId) {
    if (!Array.isArray(list)) return;
    // The reply may hold every entry or only this app's; handle both.
    const apps = new Set(list.map((t) => t.appId).concat(appId));
    this.entries = (this.entries || []).filter((t) => !apps.has(t.appId)).concat(list);
  },

  async set(appId, host, enabled, input) {
    try { this.take(await invoke('set_trust', { appId, host, enabled }), appId); }
    catch (e) { if (input) input.checked = !enabled; ctx.toast(errText(e)); }
    this.render();
  },

  async remove(appId, host) {
    try { this.take(await invoke('remove_trust', { appId, host }), appId); ctx.toast(`${host} removed.`); }
    catch (e) { ctx.toast(errText(e)); }
    this.render();
  },

  async add(form) {
    const appId = form.closest('.tr-app').dataset.app, msg = $('.tr-msg', form), input = form.elements.host;
    const host = cleanHost(input.value);
    msg.textContent = '';
    if (!input.value.trim()) return input.focus();
    if (!host) { msg.textContent = 'That does not look like a place on the internet. Try example.com or *.example.com.'; return input.focus(); }
    try {
      this.take(await invoke('set_trust', { appId, host, enabled: true }), appId);
      input.value = '';
    } catch (e) { msg.textContent = errText(e); }
    this.render();
  },
};

// ---------- Apps ----------

const apps = {
  busy: new Map(), errors: new Map(),
  show() { this.render(); },
  onStatus() { this.render(); },
  onEvent() {},

  stateLine(a) {
    if (!a.installed) return ['', 'Not installed'];
    if (!a.protected) return ['', 'Not protected'];
    if (a.fence === 'checking') return ['warn', 'Checking protection…'];
    if (a.fence !== 'up') return ['warn', 'Needs setup'];
    if (a.kind === 'agent') return ['on', 'Protected. Open a protected terminal to use it.'];
    return a.running ? ['on', 'Protected and running'] : ['idle', 'Protected, not running'];
  },
  whyNot(a) {
    if (!a.installed) return `Install ${a.name} first.`;
    if (!a.protected) return `Turn on protection for ${a.name} to use this.`;
    return 'Finish setup first.';
  },

  render() {
    const list = ctx.status().apps;
    patch(body, `<div class="ap"><h2>Apps</h2><p class="lead">Protected apps run in their own private space and can only reach what you trust.</p>
      ${list.map((a) => this.card(a)).join('') || '<p class="muted">No AI apps were found on this computer yet.</p>'}</div>`);
  },

  card(a) {
    const [cls, words] = this.stateLine(a), busy = this.busy.get(a.id), err = this.errors.get(a.id);
    const off = !a.canStart, why = off ? `<span class="ap-why">${esc(this.whyNot(a))}</span>` : '';
    let controls;
    if (a.kind === 'model') {
      controls = a.running
        ? `<button class="btn" data-do="app-stop" data-key="run-${esc(a.id)}"${busy ? ' disabled' : ''}>${icon('stop')}${busy ? 'Stopping…' : 'Stop'}</button>`
        : `<button class="btn" data-do="app-start" data-key="run-${esc(a.id)}"${off || busy ? ' disabled' : ''}>${icon('play')}${busy ? 'Starting…' : 'Start'}</button>${why}`;
    } else {
      controls = `<button class="btn" data-do="app-terminal" data-key="term-${esc(a.id)}"${off ? ' disabled' : ''}>${icon('terminal')}Open protected terminal</button>
        <button class="btn" data-do="app-workspace" data-key="ws-${esc(a.id)}"${a.workspace ? '' : ' disabled'}>${icon('folder')}Open workspace</button>${why}`;
    }
    const t = a.today;
    return `<section class="ap-card" data-app="${esc(a.id)}">
      <div class="ap-head">${appTile(a.icon, a.name, 36)}
        <div class="grow"><div class="ap-name">${esc(a.name)}</div>
          <div class="ap-state"><span class="state ${cls}"></span>${words}</div></div>
        <label class="ap-protect"><span>Protected</span><span class="switch green"><input type="checkbox" role="switch" data-protect data-key="p-${esc(a.id)}" aria-label="Protect ${esc(a.name)}"${a.protected ? ' checked' : ''}${a.installed ? '' : ' disabled'}><i></i></span></label></div>
      <div class="ap-row">${controls}</div>
      ${a.workspace ? `<div class="ap-meta">Works only inside <span class="mono sel">${esc(a.workspace)}</span></div>` : ''}
      ${err ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(err)}</span></div>` : ''}
      <div class="ap-meta num">Today: ${fmtCount(t.allowed)} allowed · ${fmtCount(t.blocked)} blocked · ${esc(fmtBytes(t.bytesOut))} sent</div>
    </section>`;
  },

  async run(appId, cmd, args = {}, slow = false) {
    this.errors.delete(appId);
    if (slow) { this.busy.set(appId, true); this.render(); }
    try { const out = await invoke(cmd, { appId, ...args }); if (out) ctx.setStatus(out); }
    catch (e) { this.errors.set(appId, errText(e)); }
    this.busy.delete(appId);
    if (visible && pane === 'apps') this.render();
  },
};

const VIEWS = { trusted, apps, logs, advanced };

// ---------- shell ----------

export function init(el, context) {
  root = el; ctx = context;
  on(root, 'click', '.st-nav button', (ev, b) => open(b.dataset.pane));
  on(root, 'change', 'input[data-trust]', (ev, i) => trusted.set(i.closest('.tr-app').dataset.app, i.dataset.trust, i.checked, i));
  on(root, 'change', 'input[data-protect]', (ev, i) => apps.run(i.closest('.ap-card').dataset.app, 'app_set_protected', { protected: i.checked }));
  on(root, 'submit', '.tr-add', (ev, form) => { ev.preventDefault(); trusted.add(form); });
  on(root, 'click', '[data-do]', (ev, b) => {
    const what = b.dataset.do, card = b.closest('.ap-card'), appId = card && card.dataset.app;
    if (what === 'tr-reload') trusted.load();
    else if (what === 'tr-remove') trusted.remove(b.closest('.tr-app').dataset.app, b.dataset.host);
    else if (what === 'app-start') apps.run(appId, 'app_start', {}, true);
    else if (what === 'app-stop') apps.run(appId, 'app_stop', {}, true);
    else if (what === 'app-terminal') apps.run(appId, 'app_open_terminal');
    else if (what === 'app-workspace') apps.run(appId, 'app_open_workspace');
    else if (VIEWS[pane].act) VIEWS[pane].act(what, b);
  });
  [logs, advanced].forEach((v) => v.init({ ctx, isShown: (name) => visible && pane === name, body: () => body }));
}

function build() {
  if (root.querySelector('.studio')) return;
  root.__html = null;
  root.innerHTML = `<div class="studio"><nav class="st-nav" aria-label="Studio">
    ${PANES.map(([id, label, ic]) => `<button data-pane="${id}">${icon(ic)}<span>${label}</span></button>`).join('')}</nav>
    <div class="st-body scroll" tabindex="-1"></div></div>`;
  body = $('.st-body', root);
}

function sync() {
  $$('.st-nav button', root).forEach((b) => {
    if (b.dataset.pane === pane) b.setAttribute('aria-current', 'page'); else b.removeAttribute('aria-current');
  });
  body.__html = null;
  body.scrollTop = 0;
  VIEWS[pane].show();
}

/** Switch pane; safe to call before the tab is shown. */
export function open(name) {
  if (!VIEWS[name]) return;
  pane = name;
  if (visible) sync();
}

export function show() { visible = true; build(); sync(); }
export function hide() { visible = false; }
export function onStatus() { if (visible) VIEWS[pane].onStatus(); }
export function onEvent(evt) { if (visible) VIEWS[pane].onEvent(evt); }
