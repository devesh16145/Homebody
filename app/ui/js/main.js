// Main window shell: toolbar, tabs, first-run setup. Feed, Proof and Studio are their own modules.
import { invoke, onEvent } from './api.js';
import { $, $$, esc, on, patch, initPage, errText, dot, MODE } from './util.js';
import { icon, appTile, mark } from './icons.js';
import * as feed from './feed.js';
import * as proof from './proof.js';
import * as studio from './studio.js';
import * as watch from './watch.js';

initPage();

const TABS = { feed, proof, studio };
const params = new URLSearchParams(location.search);
const el = { setup: $('#setup'), pill: $('#pill'), banner: $('#banner'), toast: $('#toast') };
let status = null;
let tab = TABS[params.get('tab')] ? params.get('tab') : 'feed';
let shown = null;                       // tab whose module is currently active
let setup = { state: 'idle', error: '', step: 1 };   // step 1 what, 2 approve, 3 how to start
let toastTimer = 0;

const ctx = {
  status: () => status,
  setStatus(s) { if (s && s.mode) { status = s; applyStatus(); } },
  go,
  toast(msg) {
    el.toast.textContent = msg;
    el.toast.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.toast.classList.remove('show'), 2600);
  },
};

// ---------- tabs ----------

function go(next, sub) {
  if (!TABS[next]) return;
  tab = next;
  if (sub) studio.open(sub);
  syncTabs();
}

function syncTabs() {
  const needsSetup = !!(status && (status.setupNeeded || setup.step === 3));
  document.documentElement.classList.toggle('needs-setup', needsSetup);
  el.setup.hidden = !needsSetup;
  $$('#tabs [role=tab]').forEach((b) => {
    const sel = b.dataset.tab === tab;
    b.setAttribute('aria-selected', String(sel));
    b.tabIndex = sel ? 0 : -1;
    $('#tab-' + b.dataset.tab).hidden = !sel;
  });
  const want = needsSetup || !status ? null : tab;
  if (want === shown) return;
  if (shown) TABS[shown].hide();
  shown = want;
  if (shown) TABS[shown].show();
}

on(document, 'click', '#tabs [role=tab]', (ev, b) => go(b.dataset.tab));

document.addEventListener('keydown', (ev) => {
  if ((ev.metaKey || ev.ctrlKey) && !ev.shiftKey && !ev.altKey && ['1', '2', '3'].includes(ev.key)) {
    ev.preventDefault();
    go(['feed', 'proof', 'studio'][Number(ev.key) - 1]);
  }
  const t = ev.target.closest && ev.target.closest('#tabs [role=tab]');
  if (t && (ev.key === 'ArrowLeft' || ev.key === 'ArrowRight')) {
    const order = ['feed', 'proof', 'studio'];
    const next = order[(order.indexOf(tab) + (ev.key === 'ArrowRight' ? 1 : 2)) % 3];
    go(next);
    $('#tabbtn-' + next).focus();
  }
});

// ---------- status ----------

function applyStatus() {
  const label = status.setupNeeded ? 'Needs setup' : status.lockdown ? 'Everything blocked' : MODE[status.mode] || '';
  patch(el.pill, `${dot(status.indicator)}<span>${esc(label)}</span>`);
  el.pill.title = status.sentence;
  patch(el.banner, status.lockdown && !status.setupNeeded
    ? `<div class="lockbar" role="alert">${icon('lock')}<span><b>Everything is blocked.</b> Nothing can leave until you release it.</span>
       <button class="btn sm" data-act="release">Release</button></div>` : status.setupNeeded ? '' : (watch.barHtml(status) || installBar()));
  watch.onStatus(status);
  if (status.setupNeeded || setup.step === 3) renderSetup();
  syncTabs();
  Object.values(TABS).forEach((m) => m.onStatus(status));
}

// ---------- "not in Applications yet" ----------

let install = null, installBusy = false;
const installDismissed = () => { try { return localStorage.getItem('sai.install.later') === '1'; } catch (e) { return false; } };
function installBar() {
  if (!install || !install.canInstall || installDismissed()) return '';
  return `<div class="watchbar done" role="status">${icon('info')}<span><b>Sandbox-ai is not in your Applications folder yet.</b> Moving it there keeps it in one place and easy to find.</span>
    <button class="btn sm primary" data-act="install-go"${installBusy ? ' disabled' : ''}>${installBusy ? 'Moving…' : 'Move to Applications'}</button>
    <button class="btn sm" data-act="install-later">Not now</button></div>`;
}
on(document, 'click', '[data-act="install-later"]', () => { try { localStorage.setItem('sai.install.later', '1'); } catch (e) { /* private mode */ } if (status) applyStatus(); });
on(document, 'click', '[data-act="install-go"]', async () => {
  installBusy = true; applyStatus();
  try { await invoke('install_and_restart'); } catch (e) { installBusy = false; ctx.toast(errText(e)); applyStatus(); }
});
invoke('get_install').then((i) => { install = i; if (status) applyStatus(); }).catch(() => {});

on(document, 'click', '[data-act="release"]', async () => {
  try { ctx.setStatus(await invoke('set_lockdown', { on: false })); } catch (e) { ctx.toast(errText(e)); }
});

// ---------- setup / first run ----------

const KIND = { model: 'Runs AI models on this computer', agent: 'AI assistant, command line', window: 'Windowed app, network fenced' };

function renderSetup() {
  const reason = status.setupReasons[0];
  const found = status.apps.filter((a) => a.installed);
  const waiting = setup.state === 'waiting';
  // Coming back to repair or update protection is one screen, not a tour.
  const step = setup.step === 3 ? 3 : reason ? 2 : setup.step;
  const dots = reason ? '' : `<div class="setup-steps" aria-label="Step ${step} of 3">${[1, 2, 3].map((n) => `<i${n === step ? ' class="on"' : ''}></i>`).join('')}</div>`;
  let body;
  if (step === 1) {
    const apps = found.length ? found.map((a) => `<label class="found">${appTile(a.icon, a.name, 28)}
        <span class="grow"><span class="name">${esc(a.name)}</span><br><span class="kind">${KIND[a.kind] || ''}</span></span>
        <span class="check"><input type="checkbox" data-protect="${esc(a.id)}" data-key="protect-${esc(a.id)}" aria-label="Protect ${esc(a.name)}"${a.protected ? ' checked' : ''}><i>${icon('check')}</i></span></label>`).join('')
      : '<p class="muted">No AI apps were found yet. You can still turn on protection, and add apps later in Studio.</p>';
    body = `<h1>Keep your AI on your computer.</h1>
      <p class="lead">Your AI apps keep working, but they can only reach the places you trust. Everything else stays here, and you get a record that proves it.</p>
      <div class="setup-card one"><h2>Found on this computer</h2>${apps}</div>
      <div class="setup-go"><button class="btn primary lg" data-act="setup-next" data-key="setup-next">Continue</button>
        <p class="cap">Untick anything you would rather leave as it is. You can change this later.</p></div>`;
  } else if (step === 2) {
    body = `<h1>${reason ? 'Protection needs your approval' : 'One approval, and it is on.'}</h1>
      <p class="lead">${reason ? 'Nothing is unprotected: protected apps simply cannot be started until this is done.' : 'To fence an app, your Mac has to agree. It asks for your password once; Sandbox-ai never sees it.'}</p>
      ${reason ? `<div class="notice warn" role="status">${icon('alert')}<span>${esc(reason)}</span></div>` : ''}
      <div class="setup-card one"><h2>What it does</h2><ul class="does">
        <li>${icon('check')}<span>Creates a private space for each protected app, away from your files</span></li>
        <li>${icon('check')}<span>Makes sure they can only reach what you trust</span></li>
        <li>${icon('check')}<span>Keeps it that way after restarts</span></li></ul>
        <p class="cap">Studio → Advanced shows exactly what is run, and can undo all of it.</p></div>
      <div class="setup-go">
        <button class="btn primary lg${waiting ? ' waiting' : ''}" data-act="run-setup" data-key="run-setup"${waiting ? ' disabled' : ''}>${waiting ? 'Waiting for your approval…' : 'Turn on protection'}</button>
        ${setup.state === 'error'
          ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(setup.error)} You can try again whenever you are ready.</span></div>`
          : `<p class="cap">${waiting ? 'Your Mac is asking for your password in its own window.' : 'Your Mac will ask for your password once. Nothing else is needed.'}</p>`}
        ${reason || waiting ? '' : '<button class="link" data-act="setup-back" data-key="setup-back">Back</button>'}
      </div>`;
  } else {
    body = `<h1>Protection is on. How do you want to start?</h1>
      <p class="lead">You can switch between these at any time from the menu bar.</p>
      <div class="setup-cols">
        <button class="setup-card choice" data-act="start-private" data-key="start-private"><h2>Private <span class="tag">Recommended</span></h2>
          <p>Only places you trust can be reached. The first time an app wants anything else, you are asked in plain words.</p></button>
        <button class="setup-card choice" data-act="start-watch" data-key="start-watch"><h2>Watch for an hour</h2>
          <p>Everything is allowed and written down. After an hour Private comes on, and you see what your AI contacted and choose what to trust.</p></button>
      </div>`;
  }
  patch(el.setup, `<div class="setup-in">${mark(48)}${body}${dots}</div>`);
}

async function runSetup() {
  const firstTime = !status.setupReasons[0];
  setup = { ...setup, state: 'waiting', error: '' };
  renderSetup();
  try {
    const res = await invoke('run_setup');
    if (!res || !res.ok) throw (res && res.message) || 'Setup did not finish.';
    setup = { state: 'idle', error: '', step: firstTime ? 3 : 1 };
    tab = 'feed';
    ctx.setStatus(await invoke('get_status'));
  } catch (e) {
    setup = { ...setup, state: 'error', error: errText(e) };
    if (status.setupNeeded) renderSetup();
  }
}

async function startWith(mode) {
  try { ctx.setStatus(await invoke('set_mode', mode === 'watch' ? { mode, minutes: 60 } : { mode })); } catch (e) { ctx.toast(errText(e)); }
  setup = { state: 'idle', error: '', step: 1 };
  applyStatus();
}

on(el.setup, 'click', '[data-act="setup-next"]', () => { setup.step = 2; renderSetup(); });
on(el.setup, 'click', '[data-act="setup-back"]', () => { setup.step = 1; renderSetup(); });
on(el.setup, 'click', '[data-act="start-private"]', () => startWith('private'));
on(el.setup, 'click', '[data-act="start-watch"]', () => startWith('watch'));
on(el.setup, 'click', '[data-act="run-setup"]', runSetup);
on(el.setup, 'change', 'input[data-protect]', async (ev, input) => {
  try { ctx.setStatus(await invoke('app_set_protected', { appId: input.dataset.protect, protected: input.checked })); }
  catch (e) { input.checked = !input.checked; ctx.toast(errText(e)); }
});

// ---------- boot ----------

Object.entries(TABS).forEach(([name, m]) => m.init($('#tab-' + name), ctx));
watch.init(ctx);
if (params.get('pane')) studio.open(params.get('pane'));

onEvent((evt) => {
  if (evt.type === 'status') return ctx.setStatus(evt.data);
  if (evt.type === 'navigate') return go(evt.data.tab);
  if (status) Object.values(TABS).forEach((m) => m.onEvent(evt));
});

async function boot() {
  try { ctx.setStatus(await invoke('get_status')); }
  catch (e) {
    patch($('#tab-feed'), `<div class="empty"><div class="empty-art">${icon('alert')}</div><h2>Cannot reach the app right now</h2>
      <p>${esc(errText(e))}</p><button class="btn" data-act="reboot">Try again</button></div>`);
  }
}
on(document, 'click', '[data-act="reboot"]', boot);
boot();
