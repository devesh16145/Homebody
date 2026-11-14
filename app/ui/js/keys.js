// Keys on an agent's page: Homebody keeps the key, the agent gets a stand-in.
//
// The job is one sentence: "let this agent use my key without holding it". So the card asks
// for three things only: whose key, the key, and (already filled in when Homebody knows the
// key) the place it is for. The key is typed into a password field, sent to the engine once,
// and never comes back: after that this page knows its name and last four characters.

import { invoke } from './api.js';
import { esc, errText } from './util.js';
import { icon } from './icons.js';

const info = new Map();   // appId -> get_keys answer (known names, whether keys can be kept)
let adding = false;
let form = { name: '', other: '', value: '', places: '' };
let busy = '';
let error = '';
let shown = null;
let rerender = () => {};

export function init(render) { rerender = render; }

export async function load(appId) {
  shown = appId;
  try { info.set(appId, await invoke('get_keys', { appId })); }
  catch { info.delete(appId); }
  if (shown === appId) rerender();
}

export function leave() { shown = null; adding = false; error = ''; busy = ''; form = { name: '', other: '', value: '', places: '' }; }

const whose = (k) => (k.maker ? `${k.maker} key` : k.name);

function row(k) {
  return `<li class="hm-row"><span class="ky-ic" aria-hidden="true">${icon('lock')}</span>
    <span class="grow"><b>${esc(whose(k))} <span class="mono ky-hint">${esc(k.hint)}</span></b>
      <small>Goes only to ${esc(k.hosts.join(', '))}${k.maker ? ` · <span class="mono">${esc(k.name)}</span>` : ''}</small></span>
    <button class="cap quiet sm" data-do="key-remove" data-name="${esc(k.name)}" data-key="key-remove-${esc(k.name)}"${busy ? ' disabled' : ''}>${busy === k.name ? 'Removing…' : 'Remove'}</button></li>`;
}

function formHtml(a, known) {
  const custom = form.name === '*';
  const options = known.map((k) => `<option value="${esc(k.name)}"${form.name === k.name ? ' selected' : ''}>${esc(k.maker)} (${esc(k.name)})</option>`).join('');
  return `<form class="ky-form" data-key="ky-form">
    <label>Whose key<select name="name" data-key="ky-name" aria-label="Whose key">${options}<option value="*"${custom ? ' selected' : ''}>Another one…</option></select></label>
    ${custom ? `<label>The name ${esc(a.name)} reads it from<input name="other" value="${esc(form.other)}" placeholder="MY_API_KEY" data-key="ky-other" autocomplete="off" autocapitalize="characters" spellcheck="false"></label>` : ''}
    <label>The key<input name="value" type="password" value="${esc(form.value)}" placeholder="Paste it here" data-key="ky-value" autocomplete="off" spellcheck="false"></label>
    <label>The only place it may go<input name="places" value="${esc(form.places)}" placeholder="api.example.com" data-key="ky-places" autocomplete="off" spellcheck="false"></label>
    ${a.inspect ? '' : `<p class="hm-sub">This switches on Look inside for ${esc(a.name)}: Homebody can only put a key into a connection it looks inside.</p>`}
    <div class="row"><button class="cap tide" type="submit"${busy ? ' disabled' : ''}>${busy === 'add' ? 'Keeping…' : 'Keep this key'}</button><button class="cap quiet" type="button" data-do="key-cancel">Cancel</button></div>
  </form>`;
}

export function section(a) {
  const i = info.get(a.id);
  if (a.kind !== 'agent' || !a.protected || !i || !i.canKeep) return '';
  const keys = a.keys || i.keys || [];
  const head = keys.length
    ? `<ul class="hm-list">${keys.map(row).join('')}</ul>`
    : '';
  const pitch = `<div class="hm-set"><span class="grow"><b>${keys.length ? 'Keep another key' : `Keep ${esc(a.name)}’s key out of its hands`}</b>
      <small>${esc(a.name)} gets a stand-in. The real key stays with Homebody and is put in on the way out, only for the place it is for.</small></span>
      <span class="bx-act"><button class="cap" data-do="key-add" data-key="key-add">${icon('plus')}Add a key</button></span></div>`;
  return `<section class="hm-block"><h2>Keys</h2>
    ${error ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(error)}</span></div>` : ''}
    ${head}
    ${adding ? formHtml(a, i.known || []) : pitch}
    ${keys.length ? `<p class="hm-sub">A key you add or remove reaches ${esc(a.name)} the next time you open it.</p>` : ''}
  </section>`;
}

function placesFor(appId, name) {
  const k = ((info.get(appId) || {}).known || []).find((x) => x.name === name);
  return k ? k.hosts.join(', ') : '';
}

/** Typing and choosing in the form. Returns true when the page must be drawn again. */
export function onInput(ev) {
  const t = ev.target;
  if (!t.closest || !t.closest('.ky-form')) return false;
  if (t.name === 'name') {
    form.name = t.value;
    form.places = placesFor(shown, t.value);
    return true;
  }
  if (t.name in form) form[t.name] = t.value;
  return false;
}

export async function submit(appId) {
  const name = (form.name === '*' ? form.other : form.name).trim().toUpperCase();
  const hosts = form.places.split(/[\s,]+/).map((h) => h.trim()).filter(Boolean);
  error = '';
  if (!name) error = 'Say which name the app reads this key from.';
  else if (!form.value.trim()) error = 'Paste the key.';
  else if (!hosts.length) error = 'Say which place this key is for. It will be sent there and nowhere else.';
  if (error) return rerender();
  busy = 'add'; rerender();
  try {
    await invoke('key_set', { appId, name, value: form.value.trim(), hosts });
    adding = false;
    form = { name: '', other: '', value: '', places: '' };
  } catch (e) { error = errText(e); }
  busy = '';
  await load(appId);
}

/** Clicks on [data-do="key-…"]. */
export async function act(what, appId, button) {
  error = '';
  if (what === 'key-add') {
    const known = (info.get(appId) || {}).known || [];
    const first = known.length ? known[0].name : '*';
    adding = true;
    form = { name: first, other: '', value: '', places: placesFor(appId, first) };
    return rerender();
  }
  if (what === 'key-cancel') { adding = false; form = { name: '', other: '', value: '', places: '' }; return rerender(); }
  if (what === 'key-remove') {
    const name = button.dataset.name;
    if (!confirm(`Stop keeping ${name}? The app will no longer be handed it.`)) return;
    busy = name; rerender();
    try { await invoke('key_remove', { appId, name }); } catch (e) { error = errText(e); }
    busy = '';
    await load(appId);
  }
}
