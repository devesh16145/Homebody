// Prompt banner: shows the oldest unanswered ask with exactly three answers.
import { invoke, onEvent } from './api.js';
import { $, esc, on, initPage, errText, drain, secondsLeft, verbChip } from './util.js';
import { appTile } from './icons.js';

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
  root.innerHTML = `<section class="p-card" role="alertdialog" aria-labelledby="p-title" aria-describedby="p-body">
    <div class="p-top">${appTile(ask.appIcon, ask.appName, 36)}
      <div class="p-text"><div class="p-title" id="p-title" title="${esc(ask.title)}">${esc(ask.title)}</div>
        <div class="p-body" id="p-body">${esc(ask.body)}</div></div>
      <div class="p-side"><span class="num" id="p-secs" aria-label="Seconds left"></span><span class="p-more" id="p-more"></span></div></div>
    <div class="p-btns" role="group" aria-label="Your answer">
      <button class="btn" data-choice="once">Allow once</button>
      <button class="btn" data-choice="always">Always allow</button>
      <button class="btn" data-choice="block">Block</button></div>
    <div class="p-bar"><i></i></div></section>`;
  drain($('.p-bar i', root), ask);
  updateCount(ask);
  // Silence means Block, so Block is where the keyboard starts.
  $('[data-choice="block"]', root).focus({ preventScroll: true });
}

function updateCount(ask) {
  const secs = $('#p-secs', root), more = $('#p-more', root);
  const tick = () => { if (secs) secs.textContent = secondsLeft(ask) + ' s'; };
  tick();
  clearInterval(ticker);
  ticker = setInterval(tick, 500);
  const others = queue.length - 1;
  if (more) more.textContent = others > 0 ? `+${others} more` : ask.waiting > 1 ? `${ask.waiting} waiting` : '';
}

function renderOutcome() {
  const a = outcome.ask;
  shownId = null; idle = false;
  root.innerHTML = `<section class="p-card" role="status"><div class="p-done">${appTile(a.appIcon, a.appName, 36)}
    <div class="p-text"><div class="p-title">No answer, so it was blocked.</div>
    <div class="p-body">${esc(a.appName)} did not reach ${esc(a.host)}. You can change this in the Feed.</div></div>
    ${verbChip('blocked')}</div></section>`;
}

function showError(msg) {
  const body = $('#p-body', root);
  if (body) body.textContent = msg;
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
