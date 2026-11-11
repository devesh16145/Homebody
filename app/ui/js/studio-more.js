// Studio panes: Logs (raw receipts) and Advanced (read-only settings, health check, removal).
// Advanced is the only place where technical words may appear.
import { invoke } from './api.js';
import { $, esc, patch, errText, fmtCount } from './util.js';
import { icon } from './icons.js';

// ---------- Logs ----------

export const logs = {
  lines: null, error: '', check: null, state: 'idle', saved: '', soon: 0,
  init(env) { this.env = env; },
  show() { this.render(); this.load(true); },
  onStatus() {},
  onEvent({ type }) {
    if (type !== 'feed') return;
    clearTimeout(this.soon);
    this.soon = setTimeout(() => { if (this.env.isShown('logs')) this.load(false); }, 1200);
  },

  async load(toEnd) {
    try { this.lines = await invoke('get_receipts', { limit: 500 }); this.error = ''; }
    catch (e) { this.error = errText(e); }
    if (!this.env.isShown('logs')) return;
    const pre = $('.log-lines', this.env.body());
    const pinned = toEnd || !pre || pre.scrollHeight - pre.scrollTop - pre.clientHeight < 40;
    this.render();
    const next = $('.log-lines', this.env.body());
    if (next && pinned) next.scrollTop = next.scrollHeight;
  },

  render() {
    const c = this.check, busy = this.state !== 'idle';
    let status = '';
    if (this.state === 'verifying') status = '<span class="log-status">Checking every receipt…</span>';
    else if (this.saved) status = `<span class="log-status ok">${icon('check')}Saved to Downloads <span class="mono sel">${esc(this.saved)}</span></span>`;
    else if (c && c.ok) status = `<span class="log-status ok num">${icon('check')}Chain intact · ${fmtCount(c.count)} ${c.count === 1 ? 'receipt' : 'receipts'}</span>`;
    else if (c) status = `<span class="log-status bad">${icon('alert')}${esc(c.error || 'The record does not add up.')}</span>`;
    let content;
    if (this.error && !this.lines) content = `<div class="notice error">${icon('alert')}<span>${esc(this.error)}</span><button class="link" data-do="lg-reload">Try again</button></div>`;
    else if (!this.lines) content = '<div class="code log-lines" aria-busy="true">' + '<span class="skel" style="margin:7px 0;width:92%"></span>'.repeat(8) + '</div>';
    else content = `<ol class="code log-lines sel scroll" aria-label="Receipts, newest last">${this.lines.map((l) => `<li>${esc(l)}</li>`).join('')}</ol>`;
    patch(this.env.body(), `<div class="logs"><h2>Logs</h2>
      <p class="lead">Every decision is written down as a signed receipt, each one linked to the one before. Newest at the bottom. Nothing here leaves this computer unless you export it.</p>
      <div class="log-bar">
        <button class="btn" data-do="lg-verify" data-key="lg-verify"${busy ? ' disabled' : ''}>${this.state === 'verifying' ? 'Verifying…' : 'Verify'}</button>
        <button class="btn" data-do="lg-export" data-key="lg-export"${busy ? ' disabled' : ''}>${this.state === 'saving' ? 'Exporting…' : 'Export the whole log'}</button>
        ${status}</div>
      ${content}</div>`);
  },

  async act(what) {
    if (what === 'lg-reload') return this.load(true);
    if (what !== 'lg-verify' && what !== 'lg-export') return;
    this.saved = '';
    this.state = what === 'lg-verify' ? 'verifying' : 'saving';
    this.render();
    try {
      if (what === 'lg-verify') this.check = await invoke('verify_chain');
      else this.saved = (await invoke('export_log')).path;
    } catch (e) { this.check = { ok: false, count: 0, checkedAt: '', error: errText(e) }; }
    this.state = 'idle';
    if (this.env.isShown('logs')) this.render();
  },
};

// ---------- Advanced ----------

const FENCE = {
  up: ['good', 'check', 'Protection is in place for every protected app.'],
  down: ['warn', 'alert', 'Protection is not in place right now. Run setup again to restore it.'],
  not_installed: ['warn', 'alert', 'Protection has not been set up on this computer.'],
  checking: ['', 'info', 'Checking…'],
};

export const advanced = {
  text: null, error: '', checked: false, checking: false, confirm: false, removing: false, removeError: '',
  init(env) { this.env = env; },
  show() { this.confirm = false; this.removeError = ''; this.render(); this.load(); },
  onStatus() {
    this.render();
    clearTimeout(this.soon);
    this.soon = setTimeout(() => { if (this.env.isShown('advanced')) this.load(); }, 500);
  },
  onEvent() {},

  async load() {
    try { this.text = await invoke('get_policy_text'); this.error = ''; }
    catch (e) { this.error = errText(e); }
    try { this.setupText = await invoke('get_setup_script'); } catch (e) { this.setupText = null; }
    try { this.login = (await invoke('get_login_item')).on; } catch (e) { this.login = null; }
    try { this.install = await invoke('get_install'); } catch (e) { this.install = null; }
    if (this.env.isShown('advanced')) this.render();
  },

  render() {
    const s = this.env.ctx.status();
    let health = '';
    if (this.checking) health = '<span class="log-status">Checking…</span>';
    else if (this.checked) {
      const [cls, ic, words] = FENCE[s.fence] || FENCE.down;
      health = `<div class="notice ${cls}" role="status">${icon(ic)}<span>${words}</span></div>`;
    }
    const code = this.text != null ? `<pre class="code ad-code sel scroll" tabindex="0" aria-label="Settings, read-only">${esc(this.text)}</pre>`
      : this.error ? `<div class="notice error">${icon('alert')}<span>${esc(this.error)}</span></div>`
      : '<div class="code ad-code" aria-busy="true">' + '<span class="skel" style="margin:7px 0;width:60%"></span>'.repeat(6) + '</div>';
    const removal = this.confirm
      ? `<div class="ad-confirm" role="alertdialog" aria-labelledby="ad-q">
          <p id="ad-q"><b>Remove protection from this computer?</b></p>
          <p>Your AI apps will be able to reach anything again and the Private streak ends. The record of past activity stays on this computer. Your Mac will ask for your password.</p>
          ${this.removeError ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(this.removeError)}</span></div>` : ''}
          <div class="row"><button class="btn" data-do="ad-cancel" data-key="ad-cancel"${this.removing ? ' disabled' : ''}>Cancel</button>
          <button class="btn danger solid" data-do="ad-remove" data-key="ad-remove"${this.removing ? ' disabled' : ''}>${this.removing ? 'Waiting for your approval…' : 'Remove protection'}</button></div></div>`
      : '<button class="btn danger" data-do="ad-ask" data-key="ad-ask">Remove protection from this computer</button>';
    patch(this.env.body(), `<div class="ad"><h2>Advanced</h2>
      <p class="lead">The technical details behind the switches. You never need to change anything here.</p>
      ${this.install && this.install.path ? `<section><h3>Where Sandbox-ai lives</h3>
        <div class="ad-switch"><span class="grow"><b>${this.install.installed ? 'In your Applications folder' : 'Not in your Applications folder yet'}</b><br><span class="muted mono sel">${esc(this.install.path)}</span></span>
          ${this.install.canInstall ? `<button class="btn" data-do="ad-install" data-key="ad-install"${this.installing ? ' disabled' : ''}>${this.installing ? 'Moving…' : 'Move to Applications'}</button>` : ''}</div>
        ${this.install.canInstall ? '<p class="muted ad-note">Copies the app into Applications and reopens it from there. Protected apps that are running are stopped first, and your launchers keep working.</p>' : ''}
        ${this.installError ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(this.installError)}</span></div>` : ''}</section>` : ''}
      <section><h3>Servers an agent starts itself</h3>
        <div class="ad-switch"><span class="grow"><b>${s.localServers ? 'Agents can reach their own local servers' : 'Agents cannot reach local servers yet'}</b><br>
          <span class="muted">${s.localServers
            ? 'A protected agent can start a server on this computer, such as a preview of the site it is building, and connect to it. It still cannot reach anyone else’s.'
            : 'An agent that starts a server on this computer, such as a preview of the site it is building, cannot connect to it. Some agents need this to run at all. Turning it on tests, on this Mac, that an agent can reach only its own servers before allowing it.'}</span></span>
          ${s.localServers ? '' : `<button class="btn" data-do="ad-local" data-key="ad-local"${this.localBusy ? ' disabled' : ''}>${this.localBusy ? 'Waiting for your approval…' : 'Test and turn on'}</button>`}</div>
        ${this.localNote ? `<div class="notice ${this.localNote.ok ? 'ok' : 'warn'}" role="status">${icon(this.localNote.ok ? 'check' : 'alert')}<span>${esc(this.localNote.text)}</span></div>` : ''}</section>
      ${this.login == null ? '' : `<section><h3>Start-up</h3>
        <label class="ad-switch"><span class="grow"><b>Open Sandbox-ai when I log in</b><br><span class="muted">So protected apps are ready, and the menu-bar icon is there, without you starting anything.</span></span>
          <span class="switch green"><input type="checkbox" role="switch" data-login data-key="ad-login" aria-label="Open Sandbox-ai when I log in"${this.login ? ' checked' : ''}><i></i></span></label></section>`}
      <section><h3>Health</h3>
        <div class="row"><button class="btn" data-do="ad-check" data-key="ad-check"${this.checking ? ' disabled' : ''}>Check protection now</button></div>${health}</section>
      <section><h3>Settings <span class="tag">Read-only</span></h3>${code}</section>
      ${this.setupText ? `<section><h3>What setup runs <span class="tag">Read-only</span></h3>
        <p class="muted ad-note">Exactly what your Mac runs as administrator when you turn protection on. Nothing else is ever run with those rights.</p>
        <pre class="code ad-code sel scroll" tabindex="0" aria-label="Setup script, read-only">${esc(this.setupText)}</pre></section>` : ''}
      <section><h3>Remove</h3>
        <p class="muted ad-note">Undoes everything the one-time setup created. You can set it up again at any time.</p>${removal}</section>
      <p class="ad-version faint num">Version ${esc(s.version || '')}</p></div>`);
  },

  async setLogin(on) {
    try { this.login = (await invoke('set_login_item', { on })).on; }
    catch (e) { this.env.ctx.toast(errText(e)); }
    if (this.env.isShown('advanced')) this.render();
  },

  async act(what) {
    if (what === 'ad-local') {
      this.localBusy = true; this.localNote = null; this.render();
      try {
        const res = await invoke('run_setup');
        if (!res || !res.ok) throw (res && res.message) || 'Setup did not finish.';
        const st = await invoke('get_status');
        this.env.ctx.setStatus(st);
        this.localNote = st.localServers
          ? { ok: true, text: 'Tested on this Mac and turned on: an agent reached its own server and was refused at someone else’s.' }
          : { ok: false, text: 'The test did not pass on this Mac, so it stays off. Agents keep working as before, without their own local servers.' };
      } catch (e) { this.localNote = { ok: false, text: errText(e) }; }
      this.localBusy = false;
      if (this.env.isShown('advanced')) this.render();
      return;
    }
    if (what === 'ad-install') {
      this.installing = true; this.installError = ''; this.render();
      try { await invoke('install_and_restart'); }
      catch (e) { this.installing = false; this.installError = errText(e); if (this.env.isShown('advanced')) this.render(); }
      return;
    }
    if (what === 'ad-ask') { this.confirm = true; this.removeError = ''; this.render(); const c = $('[data-do="ad-cancel"]', this.env.body()); if (c) c.focus(); return; }
    if (what === 'ad-cancel') { this.confirm = false; return this.render(); }
    if (what === 'ad-check') {
      this.checking = true; this.render();
      try { this.env.ctx.setStatus(await invoke('check_fence')); } catch (e) { this.env.ctx.toast(errText(e)); }
      this.checking = false; this.checked = true;
      if (this.env.isShown('advanced')) this.render();
      return;
    }
    if (what === 'ad-remove') {
      this.removing = true; this.removeError = ''; this.render();
      try {
        const res = await invoke('remove_setup');
        if (!res || !res.ok) throw (res && res.message) || 'Protection could not be removed.';
        this.confirm = false; this.removing = false;
        this.env.ctx.setStatus(await invoke('get_status'));
      } catch (e) { this.removing = false; this.removeError = errText(e); if (this.env.isShown('advanced')) this.render(); }
    }
  },
};
