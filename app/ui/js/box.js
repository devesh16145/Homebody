// The Box on an agent's page: run the agent in a sealed Linux machine instead of on this Mac.
//
// One card that always says the one next thing: set the Box up, put this agent in it, open
// it, or stop it. The two long steps (set-up, putting an agent in) run in the engine; this
// follows them by asking how it is going, so leaving the page loses nothing.

import { invoke } from './api.js';
import { esc, errText } from './util.js';
import { icon } from './icons.js';

const states = new Map();   // appId -> box_status for that app
let acting = '';            // the button pressed and not yet answered: 'open' | 'stop' | ...
let error = '';
let timer = 0;
let shown = null;
let rerender = () => {};

const tidy = (p) => String(p || '').replace(/^\/Users\/[^/]+\//, '~/');
const clock = (iso) => new Date(iso).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

export function init(render) { rerender = render; }

export async function load(appId) {
  shown = appId;
  clearTimeout(timer);
  try { states.set(appId, await invoke('box_status', { appId })); }
  catch { states.delete(appId); }
  if (shown !== appId) return;
  rerender();
  // Follow a long step closely, a running Box loosely (it can stop by itself).
  const st = states.get(appId);
  const every = st && st.busy ? 1500 : st && st.running ? 6000 : 0;
  if (every) timer = setTimeout(() => shown === appId && load(appId), every);
}

export function leave() { shown = null; clearTimeout(timer); error = ''; acting = ''; }

/** Whether this app's page has a Box card at all. */
function applies(a, st) {
  return a.kind === 'agent' && a.protected && !!a.workspace && st && st.supported && st.helper && st.agent && st.agent.canInstall;
}

function card(title, text, actions = '', extra = '') {
  return `<div class="hm-set bx"><span class="grow"><b>${title}</b><small>${text}</small>${extra}</span>${actions ? `<span class="bx-act">${actions}</span>` : ''}</div>`;
}

const button = (what, label, cls = '') => `<button class="cap ${cls}" data-do="box-${what}" data-key="box-${what}"${acting ? ' disabled' : ''}>${acting === what ? 'One moment…' : label}</button>`;

function busyCard(a, b) {
  const mine = b.what === 'install' && b.appId === a.id;
  const title = b.what === 'setup' ? 'Setting up the Box' : mine ? `Putting ${esc(a.name)} in the Box` : `Putting ${esc(b.appName || 'another app')} in the Box`;
  let detail = esc(b.step) + '…';
  let bar = '<div class="bx-bar" aria-hidden="true"><i></i></div>';
  if (b.what === 'setup' && b.step === 'Downloading Debian' && b.downloadedMb != null) {
    const pct = Math.min(100, Math.round((b.downloadedMb / Math.max(1, b.downloadMb)) * 100));
    detail = `Downloading Debian… ${b.downloadedMb} of about ${b.downloadMb} MB`;
    bar = `<div class="bx-bar known" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct}"><i style="width:${pct}%"></i></div>`;
  }
  return card(title, `${detail} You can leave this page; it carries on.`, '', bar);
}

export function section(a) {
  const st = states.get(a.id);
  if (!applies(a, st)) return '';
  // How the last long step ended, when it failed and was about the Box itself or this agent.
  const failed = st.last && !st.last.ok && (!st.last.appId || st.last.appId === a.id) ? st.last.message : '';
  const note = error || failed;
  let body;
  if (st.busy) body = busyCard(a, st.busy);
  else if (st.running && st.appId === a.id) {
    body = card('Running in the Box', `Since ${esc(clock(st.since))}. Of this Mac it sees only ${esc(tidy(st.share))}.`,
      button('open', `${icon('terminal')}Open a terminal`, 'tide') + button('stop', 'Stop'));
  } else if (st.running) {
    body = card(`The Box is running ${esc(st.appName || 'another app')}`, 'One Box runs at a time. Stopping it throws its disk away; the folder it shares is kept.', button('stop', 'Stop it'));
  } else if (!st.prepared) {
    body = card('Run it in the Box', `A sealed Linux machine on this Mac. Its only way out is ${esc(a.name)}’s gate, and of your files it sees one folder. Setting up downloads Debian (about ${st.downloadMb} MB), takes a few minutes and uses about 3 GB of disk.`,
      button('setup', 'Set up the Box'));
  } else if (!st.agent.installed) {
    body = card(`Put ${esc(a.name)} in the Box`, `The Box is ready but ${esc(a.name)} is not in it yet. It takes a couple of minutes, and everything comes through ${esc(a.name)}’s gate.`,
      button('install', 'Put it in'));
  } else {
    body = card('Run it in the Box', `A sealed Linux machine: no network but this gate, and of this Mac only ${esc(tidy(a.workspace))}. Every start is a fresh copy.`,
      button('open', `${icon('terminal')}Open in the Box`));
  }
  return `<section class="hm-block"><h2>The Box</h2>
    ${note ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(note)}</span>${failed && !error ? '<button class="cap quiet sm" data-do="box-dismiss">OK</button>' : ''}</div>` : ''}
    ${body}</section>`;
}

/** Clicks on [data-do="box-…"]. */
export async function act(what, appId) {
  const st = states.get(appId) || {};
  error = '';
  try {
    if (what === 'box-dismiss') { states.set(appId, await invoke('box_dismiss', { appId })); return rerender(); }
    if (what === 'box-stop' && st.running && !confirm('Stop the Box? Anything not saved in the shared folder is thrown away.')) return;
    acting = what.slice(4); rerender();
    if (what === 'box-setup') await invoke('box_setup', { appId, background: true });
    if (what === 'box-install') await invoke('box_install', { appId, background: true });
    if (what === 'box-open') await invoke('box_open', { appId });
    if (what === 'box-stop') await invoke('box_stop', {});
  } catch (e) { error = errText(e); }
  acting = '';
  await load(appId);
}
