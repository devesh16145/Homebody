// The question pop-up: the oldest unanswered question, three answers, and a ring counting down.
import { invoke, onEvent } from './api.js';
import { $, esc, on, initPage, errText, secondsLeft } from './util.js';
import { appTile, mark } from './icons.js';

initPage();

const root = $('#prompt');
const OUTCOME_MS = 1800;
let queue = [];          // unanswered asks, oldest first
let shownId = null;      // ask currently on screen
let outcome = null;      // { ask } while the "no answer" line is showing
let ticker = 0, outcomeTimer = 0;
let idle = false;        // hide_window already requested for the current empty state
const answered = new Set(); // ids answered here, in case a stale status still lists them

function render() {
  clearInterval(ticker);
  if (outcome) return renderOutcome();
  const ask = queue[0];
  if (!ask) {
    shownId = null;
    if (idle) return;
    idle = true;
    root.innerHTML = '<p class="p-idle">No question waiting.</p>';
    invoke('hide_window').catch(() => {});
    return;
  }
  idle = false;
  if (ask.id === shownId) return updateCount(ask);
  shownId = ask.id;
  const where = ask.label || ask.host;
  root.innerHTML = `<section class="p-card" role="alertdialog" aria-labelledby="p-where" aria-describedby="p-who">
    <div class="p-top">${appTile(ask.appIcon, ask.appName, 44)}
      <div class="p-text"><div class="p-who" id="p-who">${esc(ask.appName)} wants to reach</div>
        <div class="p-where" id="p-where" title="${esc(ask.title || '')}">${esc(where)}</div>
        <div class="p-host mono" title="${esc(ask.purpose || '')}">${esc(ask.host)}</div></div>
      <div class="p-ring" title="Blocked if you don't answer"><svg viewBox="0 0 34 34" aria-hidden="true"><circle class="track" cx="17" cy="17" r="14"/><circle class="left" cx="17" cy="17" r="14"/></svg><span id="p-secs" aria-label="Seconds left"></span></div>
      <span class="p-more" id="p-more" hidden></span></div>
    <div class="p-btns" role="group" aria-label="Your answer">
      <button class="cap" data-choice="block" title="Not sure? Block it. You can allow it later.">Block</button>
      <button class="cap" data-choice="once">Once</button>
      <button class="cap tide" data-choice="always">Always</button></div></section>`;
  ring(ask);
  updateCount(ask);
  // Silence means Block, so Block is where the keyboard starts.
  $('[data-choice="block"]', root).focus({ preventScroll: true });
}

/** The ring empties over the time that is left. */
function ring(ask) {
  const c = $('.p-ring .left', root);
  if (!c || !c.animate) return;
  const end = new Date(ask.expiresAt).getTime();
  const total = Math.max(1, end - new Date(ask.createdAt).getTime());
  const left = Math.max(0, end - Date.now());
  const full = 87.96;
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  c.animate([{ strokeDashoffset: full * (1 - Math.min(1, left / total)) }, { strokeDashoffset: full }],
    { duration: left, easing: reduced ? `steps(${Math.max(1, Math.ceil(left / 1000))}, end)` : 'linear', fill: 'forwards' });
}

function updateCount(ask) {
  const secs = $('#p-secs', root), more = $('#p-more', root);
  const tick = () => { if (secs) secs.textContent = secondsLeft(ask); };
  tick();
  clearInterval(ticker);
  ticker = setInterval(tick, 500);
  const others = queue.length - 1;
  if (more) { more.textContent = others > 0 ? `+${others} more` : ''; more.hidden = others <= 0; }
}

function renderOutcome() {
  const a = outcome.ask;
  shownId = null; idle = false;
  root.innerHTML = `<section class="p-card" role="status"><div class="p-done">${mark(44, 'shut', false, 'duck')}
    <div class="p-text"><div class="p-where">No answer, so it was blocked</div>
    <div class="p-who">${esc(a.appName)} didn’t reach ${esc(a.label || a.host)}. You can allow it on its page.</div></div></div></section>`;
}

function showError(msg) {
  const who = $('#p-who', root);
  if (who) who.textContent = msg;
}

async function answer(choice) {
  const ask = queue[0];
  if (!ask || outcome) return;
  queue.shift();
  answered.add(ask.id);
  render();
  try { await invoke('decide', { askId: ask.id, choice }); }
  catch (e) {
    // Already answered elsewhere is fine; anything else: put the question back.
    if (!/already/i.test(errText(e)) && secondsLeft(ask) > 0) { answered.delete(ask.id); queue.unshift(ask); shownId = null; render(); showError(errText(e)); }
  }
}

on(root, 'click', '[data-choice]', (ev, btn) => answer(btn.dataset.choice));

// Arrow keys move between the three answers; Tab, Enter and Space work natively.
root.addEventListener('keydown', (ev) => {
  if (ev.key !== 'ArrowLeft' && ev.key !== 'ArrowRight') return;
  const btns = Array.from(root.querySelectorAll('[data-choice]'));
  const i = btns.indexOf(document.activeElement);
  if (!btns.length) return;
  ev.preventDefault();
  btns[(i + (ev.key === 'ArrowRight' ? 1 : -1) + btns.length) % btns.length].focus();
});

onEvent(({ type, data }) => {
  if (type === 'ask') {
    const i = queue.findIndex((a) => a.id === data.id);
    if (i >= 0) queue[i] = data; else queue.push(data);
    queue.sort((a, b) => a.createdAt.localeCompare(b.createdAt));
    render();
  } else if (type === 'ask_resolved') {
    const ask = queue.find((a) => a.id === data.id);
    queue = queue.filter((a) => a.id !== data.id);
    if (ask && data.choice === 'timeout' && ask.id === shownId) {
      outcome = { ask };
      clearTimeout(outcomeTimer);
      outcomeTimer = setTimeout(() => { outcome = null; render(); }, OUTCOME_MS);
    }
    render();
  } else if (type === 'status' && Array.isArray(data.pending)) {
    // Status is the source of truth if an ask event was missed.
    queue = data.pending.filter((a) => !answered.has(a.id));
    render();
  }
});

invoke('get_status').then((s) => { queue = s.pending.slice(); render(); }).catch(() => render());
