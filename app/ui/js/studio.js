// Studio: everything beyond the everyday surfaces. Sidebar shell plus Trusted and Apps.
// Logs and Advanced live in studio-more.js.
import * as projects from './projects.js';
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
    try { this.inside = await invoke('get_inside_rules', {}); } catch (e) { this.inside = []; }
    if (visible && pane === 'trusted') this.render();
  },

  /** Switches for addresses inside one place, shown under that place's row. */
  insideHtml(appId, host) {
    const rows = (this.inside || []).filter((r) => r.appId === appId && r.host === host);
    if (!rows.length) return '';
    const app = ctx.status().apps.find((x) => x.id === appId);
    return `<li class="tr-inside"><div class="tr-inside-h">Inside ${esc(rows[0].hostLabel || host)}${app && !app.inspect ? ' <span class="tag">Needs Look inside on</span>' : ''}</div>
      ${rows.map((r) => `<div class="tr-row sub">
        <span class="switch sm green"><input type="checkbox" role="switch" data-inside-rule data-host="${esc(r.host)}" data-path="${esc(r.path)}" data-key="i-${esc(r.host + r.path)}" aria-label="${esc((r.label || r.path) + ' inside ' + (r.hostLabel || r.host))}"${r.enabled ? ' checked' : ''}><i></i></span>
        <div class="grow"><div class="tr-label ellipsis">${esc(r.label || r.path)}</div><div class="tr-sub ellipsis">${esc(r.purpose || (r.source === 'you' ? 'Added by you' : ''))}</div></div>
        <span class="tr-host mono sel">${esc(r.path)}</span>
        ${r.source === 'you' ? `<button class="iconbtn" data-do="tr-inside-remove" data-host="${esc(r.host)}" data-path="${esc(r.path)}" aria-label="Remove this switch" title="Remove">${icon('x')}</button>` : ''}</div>`).join('')}</li>`;
  },

  async setInside(appId, host, path, enabled, remove = false) {
    try { this.inside = await invoke(remove ? 'remove_inside_rule' : 'set_inside_rule', remove ? { appId, host, path } : { appId, host, path, enabled }); }
    catch (e) { ctx.toast(errText(e)); }
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
        ${t.source === 'you' ? `<button class="iconbtn" data-do="tr-remove" data-host="${esc(t.host)}" data-key="rm-${esc(t.host)}" aria-label="Remove ${esc(t.host)}" title="Remove">${icon('x')}</button>` : '<span class="tr-gap"></span>'}</li>${this.insideHtml(appId, t.host)}`;
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
  found: null,           // FoundApp[]: AI apps on this computer that are not protected yet
  foundBusy: null, foundError: '', launcher: new Map(),
  adding: null,          // null, or { name, program, busy, error } while the "protect another app" form is open
  removing: null,        // id of the added app awaiting "remove?" confirmation
  show() { this.render(); this.discover(); for (const a of ctx.status().apps) if (a.kind === 'agent' && a.canStart) projects.load(a.id); },

  async discover() {
    try { this.found = await invoke('discover_apps'); } catch (e) { this.found = []; }
    if (visible && pane === 'apps') this.render();
  },

  foundHtml() {
    if (!this.found || !this.found.length) return '';
    const rows = this.found.map((f) => `<div class="fd-row" data-path="${esc(f.path)}" data-name="${esc(f.name)}">
        ${appTile('generic', f.name, 28)}
        <div class="grow fd-id"><div class="fd-name">${esc(f.name)}${f.canProtect && !f.asks ? '<span class="tag">Offline only</span>' : ''}</div>
          <div class="fd-note">${esc(f.note)}</div></div>
        ${f.canProtect ? `<button class="btn sm" data-do="found-protect" data-key="fd-${esc(f.path)}"${this.foundBusy ? ' disabled' : ''}>${this.foundBusy === f.path ? 'Adding…' : 'Protect'}</button>` : ''}
      </div>`).join('');
    return `<section class="fd"><h3>Found on this Mac, not protected</h3>
      <p class="ap-meta">These open without any limits today. Protecting one gives it lighter protection: nothing it sends can skip the gate.</p>
      <div class="fd-list">${rows}</div>
      ${this.foundError ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(this.foundError)}</span></div>` : ''}</section>`;
  },

  async protectFound(row) {
    const path = row.dataset.path, name = row.dataset.name;
    this.foundBusy = path; this.foundError = ''; this.render();
    try { ctx.setStatus(await invoke('app_add', { name, program: path })); }
    catch (e) { this.foundError = errText(e); }
    this.foundBusy = null;
    await this.discover();
  },

  async showLauncher(appId) {
    this.errors.delete(appId);
    try { const res = await invoke('app_launcher', { appId }); this.launcher.set(appId, res.path); }
    catch (e) { this.errors.set(appId, errText(e)); }
    if (visible && pane === 'apps') this.render();
  },
  onStatus() { this.render(); },
  onEvent() {},

  stateLine(a) {
    if (a.problem) return ['warn', 'Installed where protected apps cannot reach it'];
    if (!a.installed) return ['', 'Not installed'];
    if (!a.protected) return ['', 'Not protected'];
    if (a.fence === 'checking') return ['warn', 'Checking protection…'];
    if (a.fence !== 'up') return ['warn', 'Needs setup'];
    if (a.kind === 'agent') return ['on', 'Protected. Open a protected terminal to use it.'];
    if (a.kind === 'window') return a.running ? ['on', 'Open, with its network fenced'] : ['idle', 'Ready. Open it from here to use it protected.'];
    return a.running ? ['on', 'Protected and running'] : ['idle', 'Protected, not running'];
  },
  whyNot(a) {
    if (a.problem) return '';
    if (!a.installed) return `Install ${a.name} first.`;
    if (!a.protected) return `Turn on protection for ${a.name} to use this.`;
    return 'Finish setup first.';
  },

  render() {
    const all = ctx.status().apps;
    const list = all.filter((a) => a.installed || a.custom), others = all.filter((a) => !a.installed && !a.custom);
    patch(body, `<div class="ap"><h2>Apps</h2><p class="lead">Protected apps run in their own private space and can only reach what you trust.</p>
      ${list.map((a) => this.card(a)).join('') || '<p class="muted">No AI apps were found on this computer yet.</p>'}
      ${this.addHtml()}
      ${this.foundHtml()}
      ${others.length ? `<p class="ap-also"><b>Also recognised:</b> ${others.map((a) => esc(a.name)).join(', ')}. Install one and it appears here, protected from its first run.</p>` : ''}</div>`);
  },

  addHtml() {
    const f = this.adding;
    if (!f) return `<div class="ap-add"><button class="btn" data-do="app-add-open" data-key="app-add-open">${icon('plus')}Protect another app…</button>
      <span class="ap-why">A windowed app, or any program you start from the command line.</span></div>`;
    return `<form class="ap-card ap-form" data-key-form="add">
      <div class="ap-name">Protect another app</div>
      <p class="ap-meta">It gets its own private space and its own Trusted list, starting empty: the first time it contacts anything, you are asked.</p>
      <label class="ap-field"><span>Name</span><input type="text" class="field sel" name="name" data-key="add-name" value="${esc(f.name)}" placeholder="For example, My assistant" maxlength="40" autocomplete="off" spellcheck="false"></label>
      <label class="ap-field"><span>Program</span><span class="row grow"><input type="text" class="field sel grow mono" name="program" data-key="add-program" value="${esc(f.program)}" placeholder="Choose an app, or type a command like aider" autocomplete="off" spellcheck="false">
        <button type="button" class="btn" data-do="app-add-pick" data-key="add-pick">Choose…</button></span></label>
      ${f.error ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(f.error)}</span></div>` : ''}
      <div class="row ap-form-go"><button type="button" class="btn" data-do="app-add-cancel" data-key="add-cancel">Cancel</button>
        <button class="btn primary" data-key="add-go"${f.busy ? ' disabled' : ''}>${f.busy ? 'Adding…' : 'Add'}</button>
        <span class="ap-why">A command-line program needs your Mac password once, for its private space. A windowed app does not.</span></div>
    </form>`;
  },

  keep(form) { if (this.adding && form) { this.adding.name = form.elements.name.value; this.adding.program = form.elements.program.value; } },

  async add(form) {
    this.keep(form);
    const f = this.adding;
    if (!f || f.busy) return;
    f.busy = true; f.error = ''; this.render();
    try {
      ctx.setStatus(await invoke('app_add', { name: f.name, program: f.program }));
      this.adding = null;
    } catch (e) { f.busy = false; f.error = errText(e); }
    if (visible && pane === 'apps') this.render();
  },

  async act(what, b) {
    const form = body.querySelector('.ap-form');
    if (what === 'app-add-open') { this.adding = { name: '', program: '', busy: false, error: '' }; this.render(); const i = body.querySelector('.ap-form input'); if (i) i.focus(); return; }
    if (what === 'app-add-cancel') { this.adding = null; return this.render(); }
    if (what === 'app-add-pick') {
      this.keep(form);
      try { const path = await invoke('pick_program'); if (path && this.adding) this.adding.program = path; }
      catch (e) { if (this.adding) this.adding.error = errText(e); }
      return this.render();
    }
    if (what === 'found-protect') return this.protectFound(b.closest('.fd-row'));
    const card = b.closest('.ap-card'), appId = card && card.dataset.app;
    if (what === 'app-launcher') return this.showLauncher(appId);
    if (what === 'app-remove') { this.removing = appId; return this.render(); }
    if (what === 'app-remove-no') { this.removing = null; return this.render(); }
    if (what === 'app-remove-yes') { this.removing = null; return this.run(appId, 'app_remove', {}, true); }
  },

  card(a) {
    const [cls, words] = this.stateLine(a), busy = this.busy.get(a.id), err = this.errors.get(a.id);
    const off = !a.canStart, why = off ? `<span class="ap-why">${esc(this.whyNot(a))}</span>` : '';
    let controls;
    if (a.kind === 'window') {
      controls = a.running
        ? `<button class="btn" data-do="app-stop" data-key="run-${esc(a.id)}"${busy ? ' disabled' : ''}>${icon('stop')}${busy ? 'Quitting…' : 'Quit'}</button>`
        : `<button class="btn" data-do="app-start" data-key="run-${esc(a.id)}"${off || busy ? ' disabled' : ''}>${icon('play')}${busy ? 'Opening…' : 'Open protected'}</button>${why}`;
    } else if (a.kind === 'model') {
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
      ${a.problem ? `<div class="notice warn" role="status">${icon('alert')}<span>${esc(a.problem)}</span></div>` : ''}
      <div class="ap-row">${controls}${a.canStart ? `<button class="btn" data-do="app-launcher" data-key="ln-${esc(a.id)}" title="A shortcut of its own that opens ${esc(a.name)} protected. Keep it in the Dock.">${icon('link')}Show launcher</button>` : ''}</div>
      ${this.launcher.has(a.id) ? `<div class="notice ok" role="status">${icon('check')}<span>Shown in Finder as <b>${esc(a.name)} (protected)</b>. Drag it to your Dock and use it instead of the usual icon.</span></div>` : ''}
      ${a.workspace ? `<div class="ap-meta">Works only inside <span class="mono sel">${esc(a.workspace)}</span></div>` : ''}
      ${a.canStart && a.canInspect ? `<label class="ap-inside"><span class="grow"><b>Look inside</b><br><span class="muted">${a.inspect
          ? `On. The Feed shows what ${esc(a.name)} asks for inside its connections to trusted places, and a trusted name cannot be used as cover for another site. Applies the next time it is opened.`
          : `See what ${esc(a.name)} asks for inside its connections to trusted places: which addresses and how much, never the content. Uses a certificate that exists only on this computer and that only protected apps are told about.`}</span></span>
        <span class="switch sm green"><input type="checkbox" role="switch" data-inside data-key="in-${esc(a.id)}" aria-label="Look inside ${esc(a.name)}'s connections"${a.inspect ? ' checked' : ''}><i></i></span></label>` : ''}
      ${a.kind === 'window' ? `<div class="ap-meta ap-lighter"><b>Lighter protection.</b> Nothing it sends can skip the gate: you see it and are asked, or it is refused. It can still read your files, as it always could, and only the copy you open from here is protected. To make that possible its own inner sandbox is switched off.</div>` : ''}
      ${a.kind === 'agent' && a.canStart ? projects.section(a) : ''}
      ${err ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(err)}</span></div>` : ''}
      <div class="ap-meta num ap-foot"><span>Today: ${fmtCount(t.allowed)} allowed · ${fmtCount(t.blocked)} blocked · ${esc(fmtBytes(t.bytesOut))} sent</span>
        ${a.custom ? (this.removing === a.id
          ? `<span class="row"><span>Stop protecting ${esc(a.name)}? Its files stay in its workspace.</span><button class="btn sm" data-do="app-remove-no" data-key="rm-no-${esc(a.id)}">Cancel</button><button class="btn sm danger" data-do="app-remove-yes" data-key="rm-yes-${esc(a.id)}">Remove</button></span>`
          : `<button class="link" data-do="app-remove" data-key="rm-${esc(a.id)}">Remove</button>`) : ''}</div>
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
  on(root, 'change', 'input[data-login]', (ev, i) => advanced.setLogin(i.checked));
  on(root, 'change', 'input[data-inside-rule]', (ev, i) => trusted.setInside(i.closest('.tr-app').dataset.app, i.dataset.host, i.dataset.path, i.checked));
  on(root, 'change', 'input[data-inside]', (ev, i) => apps.run(i.closest('.ap-card').dataset.app, 'app_set_inspect', { on: i.checked }));
  on(root, 'submit', '.ap-form', (ev, form) => { ev.preventDefault(); apps.add(form); });
  on(root, 'input', '.ap-form input', (ev, i) => apps.keep(i.form));
  on(root, 'click', '[data-do]', (ev, b) => {
    const what = b.dataset.do, card = b.closest('.ap-card[data-app]'), appId = card && card.dataset.app;
    if (what === 'tr-reload') trusted.load();
    else if (what === 'tr-remove') trusted.remove(b.closest('.tr-app').dataset.app, b.dataset.host);
    else if (what === 'tr-inside-remove') trusted.setInside(b.closest('.tr-app').dataset.app, b.dataset.host, b.dataset.path, false, true);
    else if (what === 'app-start') apps.run(appId, 'app_start', {}, true);
    else if (what === 'app-stop') apps.run(appId, 'app_stop', {}, true);
    else if (what === 'app-terminal') apps.run(appId, 'app_open_terminal');
    else if (what === 'app-workspace') apps.run(appId, 'app_open_workspace');
    else if (what.startsWith('proj-')) { const it = b.closest('.pj-item'); projects.act(what, appId, it && it.dataset.project); }
    else if (VIEWS[pane].act) VIEWS[pane].act(what, b);
  });
  projects.init(() => { if (visible && pane === 'apps') apps.render(); });
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
