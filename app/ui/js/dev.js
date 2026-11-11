// Review harness: owns the one shared mock and plays the part of the window manager.
import { createMock } from './mock.js';
import { SCENARIOS } from './mock-data.js';

const $ = (s) => document.querySelector(s);
const $$ = (s) => Array.from(document.querySelectorAll(s));
const params = new URLSearchParams(location.search);
const frames = $$('iframe[data-src]');
const state = {
  scenario: SCENARIOS.includes(params.get('scenario')) ? params.get('scenario') : 'private',
  theme: ['light', 'dark'].includes(params.get('theme')) ? params.get('theme') : 'auto',
  size: params.get('size') === '760x520' ? '760x520' : '980x680',
};
let noteTimer = 0;

function note(text) {
  const el = $('#dev-note');
  el.textContent = text;
  el.classList.add('show');
  clearTimeout(noteTimer);
  noteTimer = setTimeout(() => el.classList.remove('show'), 2400);
}

function setOff(name, off) {
  $('#f-' + name).classList.toggle('is-off', off);
  const show = $(`[data-show="${name}"]`), hint = $(`[data-hint="${name}"]`);
  if (show) show.hidden = !off;
  if (hint) hint.hidden = !off;
}

// What the real app's window manager would do in response to the core.
function onHost(evt) {
  if (evt.type === 'hide_window' && evt.window !== 'main') setOff(evt.window, true);
  else if (evt.type === 'show_prompt') setOff('prompt', false);
  else if (evt.type === 'open_main') {
    const win = $('.dev-main');
    win.classList.add('flash');
    setTimeout(() => win.classList.remove('flash'), 700);
  } else if (evt.type === 'note') note(evt.text);
}

function applyTheme() {
  const roots = [document.documentElement];
  frames.forEach((f) => { try { if (f.contentDocument) roots.push(f.contentDocument.documentElement); } catch (e) { /* not loaded */ } });
  roots.forEach((r) => { if (state.theme === 'auto') r.removeAttribute('data-theme'); else r.dataset.theme = state.theme; });
}

function applySize() {
  const [w, h] = state.size.split('x');
  const main = $('#f-main iframe');
  main.width = w; main.height = h;
  $('#size-label').textContent = `${w} × ${h}`;
}

function syncControls() {
  $('#scenario').value = state.scenario;
  $$('[data-theme-set]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.themeSet === state.theme)));
  $$('[data-size]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.size === state.size)));
  const q = new URLSearchParams({ scenario: state.scenario });
  if (state.theme !== 'auto') q.set('theme', state.theme);
  if (state.size !== '980x680') q.set('size', state.size);
  history.replaceState(null, '', '?' + q);
}

function start() {
  if (window.__HB_MOCK__) window.__HB_MOCK__.dispose();
  const mock = createMock(state.scenario);
  window.__HB_MOCK__ = mock;          // the iframes pick this up through window.parent
  mock.onHost(onHost);
  mock.setFlag('failSetup', $('#fail').checked);
  mock.setLive($('#live').checked);
  setOff('glance', false);
  setOff('prompt', false);
  const q = state.theme === 'auto' ? '' : '?theme=' + state.theme;
  frames.forEach((f) => {
    if (f.contentWindow && f.getAttribute('src')) f.contentWindow.location.replace(f.dataset.src + q);
    else f.src = f.dataset.src + q;
  });
  syncControls();
}

$('#scenario').innerHTML = SCENARIOS.map((s) => `<option value="${s}">${s}</option>`).join('');
$('#scenario').addEventListener('change', (e) => { state.scenario = e.target.value; start(); });
$('#reset').addEventListener('click', start);
$('#live').addEventListener('change', (e) => window.__HB_MOCK__.setLive(e.target.checked));
$('#fail').addEventListener('change', (e) => window.__HB_MOCK__.setFlag('failSetup', e.target.checked));
frames.forEach((f) => f.addEventListener('load', applyTheme));

document.addEventListener('click', (ev) => {
  const t = ev.target.closest('button');
  if (!t) return;
  if (t.dataset.sim) window.__HB_MOCK__.simulate(t.dataset.sim);
  else if (t.dataset.themeSet) { state.theme = t.dataset.themeSet; applyTheme(); syncControls(); }
  else if (t.dataset.size) { state.size = t.dataset.size; applySize(); syncControls(); }
  else if (t.dataset.show) setOff(t.dataset.show, false);
});

applyTheme();
applySize();
start();
