// "Test it" on an app's page: Homebody tries to get out as the app, and shows what happened.
//
// Protection that cannot be seen has to be taken on trust. This makes it something you can
// watch: real attempts, made as the app's private user under its own rules, each with its
// outcome. Nothing is simulated; if one got through, it says so in red.

import { invoke } from './api.js';
import { esc, errText } from './util.js';
import { icon } from './icons.js';

const results = new Map();   // appId -> last result
let running = '';
let error = '';
let rerender = () => {};

export function init(render) { rerender = render; }
export function leave() { error = ''; }

const clock = (iso) => new Date(iso).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

export function section(a) {
  if (!a.protected || a.fence !== 'up' || !['agent', 'model'].includes(a.kind)) return '';
  const r = results.get(a.id), busy = running === a.id;
  const button = `<button class="cap${r ? '' : ' tide'}" data-do="leak-run" data-key="leak-run"${busy ? ' disabled' : ''}>${icon('shield')}${busy ? 'Trying to get out…' : r ? 'Test again' : 'Test it now'}</button>`;
  // Three outcomes, never two: an attempt that could not be judged is not counted as stopped.
  const LOOK = { stopped: ['held', 'check', 'Stopped'], through: ['out', 'alert', 'Got through'], unknown: ['unsure', 'info', 'Could not tell'] };
  const rows = r ? r.cases.map((c) => { const [cls, ic, word] = LOOK[c.outcome] || LOOK.unknown; return `<li class="hm-row lt-${cls}"><span class="lt-ic" aria-hidden="true">${icon(ic)}</span>
      <span class="grow"><b>${esc(c.what)}</b></span><span class="lt-state">${word}</span></li>`; }).join('') : '';
  const verdict = !r ? '' : r.through
    ? `<div class="notice error" role="alert">${icon('alert')}<span>Something got through. Do not rely on protection for ${esc(a.name)} until this is understood; Settings → Advanced → Check now shows which part is not holding.</span></div>`
    : r.unknown
      ? `<div class="notice warn" role="alert">${icon('info')}<span>${r.unknown === 1 ? 'One attempt' : `${r.unknown} attempts`} could not be judged: it did not finish, or was ended from outside. That is not a pass. Close ${esc(a.name)} and test again.</span></div>`
      : `<p class="hm-sub">All ${r.cases.length} attempts were seen to be stopped, at ${esc(clock(r.at))}. They were made as ${esc(a.name)} itself would have to make them.</p>`;
  return `<section class="hm-block"><h2>Test it</h2>
    ${error ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(error)}</span></div>` : ''}
    <div class="hm-set bx"><span class="grow"><b>See for yourself</b><small>Homebody tries to get out as ${esc(a.name)}: straight to the internet, around the gate, into your files. You see what happens to each attempt.</small></span>
      <span class="bx-act">${button}</span></div>
    ${rows ? `<ul class="hm-list lt">${rows}</ul>` : ''}${verdict}</section>`;
}

/** Clicks on [data-do="leak-…"]. */
export async function act(what, appId) {
  if (what !== 'leak-run' || running) return;
  running = appId; error = ''; rerender();
  try { results.set(appId, await invoke('leak_test', { appId })); }
  catch (e) { error = errText(e); }
  running = '';
  rerender();
}
