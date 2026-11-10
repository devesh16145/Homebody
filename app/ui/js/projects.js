// Projects: bring a folder in for an agent, review what it changed, apply it back.
// Rendered inside an agent's card in Studio → Apps. Owns its own small state.

import { invoke } from './api.js';
import { esc, errText, fmtCount } from './util.js';
import { icon } from './icons.js';

const lists = new Map();    // appId -> ProjectInfo[]
const sets = new Map();     // appId|name -> ChangeSet
const notes = new Map();    // appId|name -> string (result of the last apply)
const errors = new Map();   // appId -> string
const busy = new Map();     // appId|name or appId -> string (what is in progress)
let open = null;            // appId|name currently under review
let confirm = null;         // appId|name awaiting "remove the copy?" confirmation
let rerender = () => {};

const key = (appId, name) => `${appId}|${name}`;
const plural = (n, one, many) => `${fmtCount(n)} ${n === 1 ? one : many}`;
const tidy = (path) => String(path || '').replace(/^\/Users\/[^/]+\//, '~/');
const KIND = { added: 'Added', modified: 'Changed', deleted: 'Deleted' };

export function init(render) { rerender = render; }

export async function load(appId) {
  try {
    const list = await invoke('get_projects', { appId });
    lists.set(appId, list || []);
    errors.delete(appId);
  } catch (e) { errors.set(appId, errText(e)); }
  rerender();
  for (const p of lists.get(appId) || []) count(appId, p.name);
}

async function count(appId, name) {
  try { sets.set(key(appId, name), await invoke('project_changes', { appId, name })); }
  catch (e) { sets.set(key(appId, name), { failed: errText(e) }); }
  rerender();
}

function waiting(set) {
  if (!set) return ['', 'Checking…'];
  if (set.failed) return ['warn', 'Could not be checked'];
  if (!set.total) return ['', 'No changes waiting'];
  return ['has', plural(set.total, 'change waiting', 'changes waiting')];
}

function review(a, p, set) {
  const k = key(a.id, p.name), doing = busy.get(k);
  if (!set || set.failed) return `<div class="pj-review"><div class="notice error">${icon('alert')}<span>${esc((set && set.failed) || 'Checking…')}</span></div></div>`;
  const rows = set.items.map((c) => `<li>
      <span class="pj-k k-${esc(c.kind)}">${KIND[c.kind] || esc(c.kind)}</span>
      <span class="pj-path mono sel">${esc(c.path)}${c.link ? `<span class="pj-link"> points to ${esc(c.link)}</span>` : ''}</span>
      ${c.conflict ? '<span class="pj-conf">You changed this too</span>' : ''}</li>`).join('');
  const clean = set.total - set.conflicts;
  return `<div class="pj-review">
    <ul class="pj-changes scroll">${rows}</ul>
    ${set.truncated ? `<p class="pj-more">Showing the first ${fmtCount(set.items.length)} of ${fmtCount(set.total)}.</p>` : ''}
    ${set.conflicts ? `<p class="pj-warn">${icon('alert')}<span>${plural(set.conflicts, 'file was', 'files were')} also changed by you since this project was brought in. Applying leaves ${set.conflicts === 1 ? 'it' : 'them'} alone.</span></p>` : ''}
    <div class="row pj-actions">
      ${clean ? `<button class="btn primary" data-do="proj-apply" data-key="pa-${esc(k)}"${doing ? ' disabled' : ''}>${doing === 'apply' ? 'Applying…' : `Apply ${plural(clean, 'change', 'changes')} to my folder`}</button>` : ''}
      ${set.conflicts ? `<button class="btn danger" data-do="proj-apply-all" data-key="pf-${esc(k)}"${doing ? ' disabled' : ''}>Apply all, replacing mine</button>` : ''}
      <button class="btn" data-do="proj-recheck" data-key="pr-${esc(k)}"${doing ? ' disabled' : ''}>Check again</button>
      <button class="btn" data-do="proj-review" data-key="pc-${esc(k)}">Close</button>
    </div>
    <p class="pj-into">Goes into <span class="mono sel">${esc(tidy(p.source))}</span>. Version history inside the copy is not brought back; changes arrive as ordinary edits for you to review.</p>
  </div>`;
}

function item(a, p) {
  const k = key(a.id, p.name), set = sets.get(k), [cls, words] = waiting(set);
  const doing = busy.get(k), note = notes.get(k);
  const gone = !p.copyExists ? 'The copy is missing.' : !p.sourceExists ? 'Your original folder is no longer there.' : '';
  const asking = confirm === k;
  return `<div class="pj-item" data-project="${esc(p.name)}">
    <div class="pj-row">
      <span class="pj-ic">${icon('folder')}</span>
      <div class="grow pj-id"><div class="pj-name">${esc(p.name)}</div><div class="pj-src mono">${esc(tidy(p.source))}</div></div>
      <span class="pj-count ${cls}">${esc(gone || words)}</span>
      <button class="btn sm" data-do="proj-term" data-key="pt-${esc(k)}"${p.copyExists ? '' : ' disabled'}>${icon('terminal')}Open terminal</button>
      <button class="btn sm" data-do="proj-review" data-key="pv-${esc(k)}"${set && set.total && !gone ? '' : ' disabled'} aria-expanded="${open === k}">Review</button>
      <button class="btn sm icon-only" data-do="proj-remove" data-key="px-${esc(k)}" aria-label="Remove ${esc(p.name)}" title="Remove the copy">${icon('x')}</button>
    </div>
    ${asking ? `<div class="pj-confirm" role="alertdialog"><span><b>Remove this copy?</b> Changes you have not applied are lost. Your own folder is not touched.</span>
      <span class="row"><button class="btn sm" data-do="proj-remove-no" data-key="pn-${esc(k)}">Cancel</button><button class="btn sm danger solid" data-do="proj-remove-yes" data-key="py-${esc(k)}"${doing ? ' disabled' : ''}>${doing === 'remove' ? 'Removing…' : 'Remove copy'}</button></span></div>` : ''}
    ${note ? `<div class="notice ok" role="status">${icon('check')}<span>${esc(note)}</span></div>` : ''}
    ${open === k ? review(a, p, set) : ''}
  </div>`;
}

/** The Projects block for one agent's card. */
export function section(a) {
  const list = lists.get(a.id), err = errors.get(a.id), adding = busy.get(a.id);
  const rows = (list || []).map((p) => item(a, p)).join('');
  return `<div class="pj">
    <div class="pj-head"><span class="pj-title">Projects</span>
      <button class="btn sm" data-do="proj-add" data-key="padd-${esc(a.id)}"${adding ? ' disabled' : ''}>${icon('plus')}${adding ? 'Bringing in…' : 'Bring in a project…'}</button></div>
    ${rows || `<p class="pj-empty">${list ? `Bring in a folder and ${esc(a.name)} works on a copy of it. Your own folder changes only when you apply the result, so nothing it does can damage your work.` : 'Loading…'}</p>`}
    ${err ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(err)}</span></div>` : ''}
  </div>`;
}

export async function act(what, appId, name) {
  const k = name ? key(appId, name) : null;
  errors.delete(appId);
  try {
    if (what === 'proj-add') {
      const path = await invoke('pick_folder');
      if (!path) return;
      busy.set(appId, 'add'); rerender();
      await invoke('project_import', { appId, path });
      busy.delete(appId);
      return load(appId);
    }
    if (what === 'proj-term') return void (await invoke('app_open_terminal', { appId, project: name }));
    if (what === 'proj-review') { open = open === k ? null : k; notes.delete(k); rerender(); if (open) count(appId, name); return; }
    if (what === 'proj-recheck') { sets.delete(k); rerender(); return count(appId, name); }
    if (what === 'proj-remove') { confirm = k; return rerender(); }
    if (what === 'proj-remove-no') { confirm = null; return rerender(); }
    if (what === 'proj-remove-yes') {
      busy.set(k, 'remove'); rerender();
      await invoke('project_remove', { appId, name });
      busy.delete(k); confirm = null; sets.delete(k); notes.delete(k); if (open === k) open = null;
      return load(appId);
    }
    if (what === 'proj-apply' || what === 'proj-apply-all') {
      busy.set(k, 'apply'); rerender();
      const res = await invoke('project_apply', { appId, name, overwrite: what === 'proj-apply-all' });
      busy.delete(k);
      const left = (res.skipped || []).length;
      notes.set(k, `${plural(res.applied, 'change', 'changes')} applied to your folder.${left ? ` ${plural(left, 'file was', 'files were')} left alone because you changed ${left === 1 ? 'it' : 'them'} too.` : ''}`);
      if (!left) open = null;
      return count(appId, name);
    }
  } catch (e) {
    if (k) busy.delete(k);
    busy.delete(appId);
    errors.set(appId, errText(e));
    rerender();
  }
}
