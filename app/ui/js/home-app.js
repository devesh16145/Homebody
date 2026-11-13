// One app's page: open it, the folders it works on, and the places it reaches.
//
// "Where it can go" is the rules and the activity in one list: each place says what happened
// there and carries its own Allow / Block switch, so the question "what is it talking to, and
// should it?" is answered without moving between screens.

import { invoke } from './api.js';
import { $, esc, patch, errText, fmtBytes, fmtCount, relTime, isToday, cleanHost, feedTime, verbChip } from './util.js';
import { icon, appTile } from './icons.js';
import * as projects from './projects.js';
import * as box from './box.js';

let env = null;
let shownId = null;
const data = new Map();     // appId -> { events, trust, inside, loadedAt }
let moreOpen = false, adding = false, addText = '', rowBusy = '', error = '';
let reloadSoon = 0;
let inited = false;

const AUTO = new Set(['around_gate', 'malformed', 'local_address']);
const tidy = (p) => String(p || '').replace(/^\/Users\/[^/]+\//, '~/');

function matches(pattern, host) {
  if (pattern === host) return true;
  return pattern.startsWith('*.') && (host.endsWith(pattern.slice(1)) || host === pattern.slice(2));
}

async function load(appId) {
  const [events, trust, inside, web] = await Promise.all([
    invoke('get_feed', { appId, limit: 500 }).catch(() => []),
    invoke('get_trust', { appId }).catch(() => []),
    invoke('get_inside_rules', { appId }).catch(() => []),
    invoke('get_ai_sites', { appId }).catch(() => null),
  ]);
  data.set(appId, { events, trust: trust.filter((t) => t.appId === appId), inside: inside.filter((r) => r.appId === appId), sites: (web && web.sites) || [], loadedAt: Date.now() });
  if (shownId === appId) env.render();
}

/** Places: every rule, plus every place it tried, each with what happened today and last. */
function places(d, askOn) {
  const rows = new Map();
  for (const t of d.trust) {
    rows.set(t.host, { key: t.host, host: t.host, label: t.label || t.host, purpose: t.purpose, rule: t, today: 0, todayBlocked: 0, sent: 0, last: null });
  }
  const auto = { today: 0, total: 0, last: null, kinds: new Map() };
  for (const e of d.events) {
    if (AUTO.has(e.reason)) {
      auto.total++;
      if (isToday(e.ts)) auto.today++;
      if (!auto.last || e.ts > auto.last) auto.last = e.ts;
      const what = e.reason === 'malformed' ? 'Address without a name' : e.reason === 'local_address' ? 'Address on this computer or network' : e.label;
      auto.kinds.set(what, (auto.kinds.get(what) || 0) + 1);
      continue;
    }
    if (!e.host) continue;
    let row = [...rows.values()].find((r) => r.rule && matches(r.rule.host, e.host)) || rows.get(e.host);
    if (!row) { row = { key: e.host, host: e.host, label: e.label || e.host, purpose: e.purpose, rule: null, today: 0, todayBlocked: 0, sent: 0, last: null }; rows.set(e.host, row); }
    if (!row.last || e.ts > row.last) row.last = e.ts;
    if (isToday(e.ts)) {
      if (e.verb === 'blocked') row.todayBlocked++; else if (e.verb === 'allowed' || e.verb === 'seen') { row.today++; row.sent += e.bytesOut || 0; }
    }
  }
  const state = (r) => (r.rule ? (r.rule.enabled ? 'allowed' : 'blocked') : askOn ? 'ask' : 'blocked');
  const list = [...rows.values()].map((r) => ({ ...r, state: state(r) }));
  const weight = (r) => (r.today || r.todayBlocked ? 0 : 1) * 10 + ({ allowed: 0, ask: 1, blocked: 2 })[r.state];
  list.sort((a, b) => weight(a) - weight(b) || (b.today + b.todayBlocked) - (a.today + a.todayBlocked) || a.label.localeCompare(b.label));
  return { list, auto };
}

function activityText(r) {
  if (r.today) return `${fmtCount(r.today)} today${r.sent ? ` · ${fmtBytes(r.sent)} out` : ''}`;
  if (r.todayBlocked) return `Stopped ${r.todayBlocked === 1 ? 'once' : fmtCount(r.todayBlocked) + '×'} today`;
  if (r.last) return relTime(r.last);
  return 'Not used yet';
}

function placeRow(appId, r) {
  const k = `${appId}|${r.host}`, b = rowBusy === k;
  const org = r.rule && r.rule.source === 'org';
  const tags = [
    org ? `<span class="tag">${esc(env.ctx.status().org || 'Organisation')}</span>` : '',
    r.rule && r.rule.recommended ? '<span class="tag">Needed</span>' : '',
    r.rule && r.rule.limitMb ? `<span class="tag">${r.rule.limitMb === 2 ? 'Downloads only' : `${r.rule.limitMb} MB/h`}</span>` : '',
  ].join('');
  const initial = esc(String(r.label).trim().charAt(0).toUpperCase());
  return `<li class="pl pl-${r.state}" title="${esc(r.purpose ? r.purpose + ' · ' + r.host : r.host)}">
    <span class="pl-ic" aria-hidden="true">${initial}</span>
    <div class="grow pl-name"><b>${esc(r.label)}</b>${tags}<small>${esc(activityText(r))}</small></div>
    ${r.state === 'ask' ? '<span class="pl-state">Asks</span>' : ''}
    <span class="switch"><input type="checkbox" data-place="${esc(r.host)}" aria-label="Allow ${esc(r.label)}"${r.state === 'allowed' ? ' checked' : ''}${b || org ? ' disabled' : ''}><i></i></span>
  </li>`;
}

function autoRow(auto) {
  if (!auto.total) return '';
  const kinds = [...auto.kinds.entries()].sort((a, b) => b[1] - a[1]).map(([k, n]) => `${k} ${n}×`).join(', ');
  return `<li class="pl pl-auto" title="${esc(kinds)}">
    <span class="pl-ic" aria-hidden="true">${icon('shield')}</span>
    <div class="grow pl-name"><b>Tricks to get around</b><small>${auto.today ? `${fmtCount(auto.today)} today` : relTime(auto.last)}</small></div>
    <span class="pl-fixed">Always stopped</span>
  </li>`;
}

/** A browser profile's AI sites, each with one switch for all the places it needs. */
function sitesHtml(a, d) {
  const rows = d.sites.map((s) => `<li class="pl ${s.on ? 'pl-allowed' : 'pl-blocked'}" title="${esc(s.places.map((p) => p.host).join(', '))}">
    <span class="pl-ic" aria-hidden="true">${esc(s.name.charAt(0))}</span>
    <div class="grow pl-name"><b>${esc(s.name)}</b><small>${esc(s.url.replace(/^https:\/\//, '').replace(/\/$/, ''))}</small></div>
    <span class="switch"><input type="checkbox" data-site="${esc(s.id)}" aria-label="Allow ${esc(s.name)}"${s.on ? ' checked' : ''}${rowBusy === `${a.id}|site:${s.id}` ? ' disabled' : ''}><i></i></span>
  </li>`).join('');
  return `<section class="hm-block"><h2>Sites</h2><ul class="pls">${rows}</ul>
    <p class="hm-sub">Pages here can’t read your home folder, only Downloads. The browser’s own warning about its sandbox is expected: Homebody holds the pages instead.</p></section>`;
}

/** One line standing for every place a browser profile starts with switched off. */
function upkeepRow(rows) {
  if (!rows.length) return '';
  const stopped = rows.reduce((n, r) => n + r.todayBlocked, 0);
  return `<li class="pl pl-auto" title="${esc(rows.map((r) => r.host).join(', '))}">
    <span class="pl-ic" aria-hidden="true">${icon('shield')}</span>
    <div class="grow pl-name"><b>Trackers and browser upkeep</b><small>${fmtCount(rows.length)} places${stopped ? ` · stopped ${fmtCount(stopped)}× today` : ''}</small></div>
    <span class="pl-fixed">Blocked</span>
  </li>`;
}

function moreHtml(a, d) {
  const allowed = d.trust.filter((t) => t.enabled);
  const insideRows = d.inside.map((r) => `<li class="hm-row"><span class="grow"><b>${esc(r.label || r.path)}</b><small>${esc(r.hostLabel)} · <span class="mono">${r.method ? esc(r.method) + ' ' : ''}${esc(r.path)}</span></small></span>
      <span class="switch sm"><input type="checkbox" data-inside="${esc(r.host)}" data-path="${esc(r.path)}" aria-label="Allow ${esc(r.label || r.path)}"${r.enabled ? ' checked' : ''}${a.inspect ? '' : ' disabled'}><i></i></span></li>`).join('');
  const limits = allowed.map((t) => `<li class="hm-row"><span class="grow"><b>${esc(t.label || t.host)}</b><small>${t.sentMb != null ? `${t.sentMb} MB sent in the last hour` : esc(t.host)}</small></span>
      <select data-limit="${esc(t.host)}" aria-label="How much ${esc(a.name)} may send to ${esc(t.label || t.host)} per hour">
        ${[['', 'No limit'], ['2', 'Downloads only'], ['20', '20 MB an hour'], ['200', '200 MB an hour']].map(([v, l]) => `<option value="${v}"${String(t.limitMb || '') === v ? ' selected' : ''}>${l}</option>`).join('')}</select></li>`).join('');
  return `<div class="hm-morebody">
    ${a.canInspect ? `<div class="hm-set"><span class="grow"><b>Look inside</b><small>See and switch single addresses. Catch keys and private files.</small></span>
        <span class="switch"><input type="checkbox" data-inspect${a.inspect ? ' checked' : ''} aria-label="Look inside ${esc(a.name)}'s connections"><i></i></span></div>
      ${a.inspect ? `<div class="hm-set"><span class="grow"><b>Stop secrets</b><small>Anything that looks like a key or token waits for you.</small></span>
        <span class="switch"><input type="checkbox" data-secrets${a.secrets ? ' checked' : ''} aria-label="Stop secrets leaving"><i></i></span></div>` : ''}
      ${insideRows ? `<h3>Addresses inside${a.inspect ? '' : ' <span class="tag">Needs Look inside</span>'}</h3><ul class="hm-list">${insideRows}</ul>` : ''}`
    : `<p class="hm-sub">Looking inside is not available for ${esc(a.name)}: it takes its trusted certificates from the Mac itself.</p>`}
    ${limits ? `<h3>Hourly limits</h3><ul class="hm-list">${limits}</ul>` : ''}
    <h3>On this Mac</h3>
    <div class="row hm-wrap">
      <button class="cap" data-do="launcher">${icon('link')}Its Dock copy</button>
      ${a.workspace ? `<button class="cap" data-do="workspace">${icon('folder')}Its space</button>` : ''}
      ${a.custom ? '<button class="cap danger" data-do="remove">Stop protecting…</button>' : ''}
    </div>
  </div>`;
}

export function render(view, a, e) {
  env = e;
  if (!inited) {
    inited = true;
    projects.init(() => { if (shownId) env.render(); });
    box.init(() => { if (shownId) env.render(); });
    // Coming back from the terminal is exactly when you want to see what the agent changed.
    window.addEventListener('focus', () => projects.refresh(shownId, 500));
    document.addEventListener('visibilitychange', () => { if (!document.hidden) projects.refresh(shownId, 500); });
    setInterval(() => { if (!document.hidden) projects.refresh(shownId, 12000); }, 4000);
  }
  if (shownId !== a.id) {
    shownId = a.id; moreOpen = false; adding = false; addText = ''; error = '';
    projects.load(a.id);
    box.load(a.id);
    load(a.id);
    view.scrollTop = 0;
  }
  const d = data.get(a.id);
  const s = env.ctx.status();
  const [cls, line] = env.appLine(a);
  const qs = s.pending.filter((q) => q.appId === a.id);
  const err = env.appErrors.get(a.id) || error;
  const { list: every, auto } = d ? places(d, s.ask !== false) : { list: [], auto: { total: 0 } };
  // A browser profile starts with a long list of trackers switched off; they are one line here.
  const isUpkeep = (r) => a.kind === 'browser' && r.rule && r.rule.source === 'preset' && !r.rule.enabled;
  const list = every.filter((r) => !isUpkeep(r)), upkeep = every.filter(isUpkeep);
  const folderBtn = a.kind === 'agent' && a.canStart
    ? `<button class="cap" data-do="proj-add" data-key="folder-${esc(a.id)}">${icon('folder')}Folder…</button>` : '';
  const askWord = s.ask !== false ? 'Anything new: you’re asked' : 'Anything new: blocked';
  const html = `<div class="hm-page hm-app" data-app="${esc(a.id)}">
    <header class="hm-apphead">
      ${appTile(a.icon, a.name, 72)}
      <div class="grow"><h1>${esc(a.name)}</h1><p class="hm-card-line ${cls}">${esc(line)}</p></div>
      <div class="hm-apphead-act">${folderBtn}${env.primaryAction(a, 'lg')}</div>
    </header>
    ${a.problem ? `<div class="notice warn" role="alert">${icon('alert')}<span>${esc(a.problem)}</span></div>` : ''}
    ${err ? `<div class="notice error" role="alert">${icon('alert')}<span>${esc(err)}</span></div>` : ''}
    ${qs.length ? `<div class="hm-qs">${qs.map((q) => env.questionHtml(q, false)).join('')}</div>` : ''}
    ${a.kind === 'agent' && projects.has(a.id) ? `<section class="hm-block hm-proj"><h2>Folders</h2>${projects.section(a)}</section>` : ''}
    ${a.kind === 'browser' && d ? sitesHtml(a, d) : ''}
    <section class="hm-block">
      <h2>${a.kind === 'browser' ? 'Every place' : 'Can reach'}</h2>
      ${d ? `${list.length || auto.total || upkeep.length ? `<ul class="pls">${list.map((r) => placeRow(a.id, r)).join('')}${upkeepRow(upkeep)}${autoRow(auto)}</ul>`
          : `<p class="hm-empty">Nothing yet. ${s.ask !== false ? 'You’re asked' : 'It’s blocked'} the first time ${esc(a.name)} tries somewhere.</p>`}
        <div class="pl-foot">${adding ? `<form class="pl-add"><input name="host" value="${esc(addText)}" placeholder="example.com" aria-label="Place to allow" data-key="pl-add" autocomplete="off" autofocus><button class="cap tide sm" type="submit">Allow</button><button class="cap quiet sm" type="button" data-do="add-cancel">Cancel</button></form>`
          : `<button class="cap sm" data-do="add-place">${icon('plus')}Add place</button>`}<span class="muted">${askWord}</span></div>`
        : '<p class="muted">Loading…</p>'}
    </section>
    ${box.section(a)}
    <section class="hm-block">
      <button class="hm-more" data-do="more" aria-expanded="${moreOpen}">${icon('sliders')}<span class="grow"><b>Options</b></span>${icon('chevron', moreOpen ? 'rot' : '')}</button>
      ${moreOpen && d ? moreHtml(a, d) : ''}
    </section>
  </div>`;
  patch(view, html);
  bind(view);
}

let bound = null;
function bind(view) {
  if (bound === view) return;
  bound = view;
  view.addEventListener('change', async (ev) => {
    const t = ev.target, app = t.closest('[data-app]');
    if (!app) return;
    const appId = app.dataset.app;
    try {
      if (t.matches('[data-place]')) {
        rowBusy = `${appId}|${t.dataset.place}`;
        await invoke('set_trust', { appId, host: t.dataset.place, enabled: t.checked });
        rowBusy = ''; await load(appId);
      } else if (t.matches('[data-site]')) {
        rowBusy = `${appId}|site:${t.dataset.site}`;
        await invoke('browser_set_site', { appId, site: t.dataset.site, on: t.checked });
        rowBusy = ''; await load(appId);
      } else if (t.matches('[data-inspect]')) env.ctx.setStatus(await invoke('app_set_inspect', { appId, on: t.checked }));
      else if (t.matches('[data-secrets]')) env.ctx.setStatus(await invoke('app_set_secrets', { appId, on: t.checked }));
      else if (t.matches('[data-inside]')) { await invoke('set_inside_rule', { appId, host: t.dataset.inside, path: t.dataset.path, enabled: t.checked }); load(appId); }
      else if (t.matches('[data-limit]')) { await invoke('set_limit', t.value ? { appId, host: t.dataset.limit, mb: Number(t.value) } : { appId, host: t.dataset.limit }); load(appId); }
      else if (t.matches('input[data-suggest]')) projects.toggleSuggestion(appId, t.closest('.pj-item').dataset.project, Number(t.dataset.suggest), t.checked);
    } catch (e) { rowBusy = ''; if (t.type === 'checkbox') t.checked = !t.checked; env.ctx.toast(errText(e)); }
  });
  view.addEventListener('input', (ev) => { if (ev.target.name === 'host' && ev.target.closest('.pl-add')) addText = ev.target.value; });
  view.addEventListener('submit', async (ev) => {
    if (!ev.target.matches('.pl-add')) return;
    ev.preventDefault();
    const host = cleanHost(addText);
    if (!host) return env.ctx.toast('Type a place like example.com.');
    try { await invoke('set_trust', { appId: shownId, host, enabled: true }); adding = false; addText = ''; load(shownId); }
    catch (e) { env.ctx.toast(errText(e)); }
  });
}

/** Clicks on [data-do] inside an app page, routed here by the shell. */
export async function act(what, b) {
  const appId = shownId;
  if (!appId) return;
  if (what.startsWith('proj-')) { const it = b.closest('.pj-item'); return projects.act(what, appId, it && it.dataset.project); }
  if (what.startsWith('box-')) return box.act(what, appId);
  if (what === 'more') { moreOpen = !moreOpen; return env.render(); }
  if (what === 'add-place') { adding = true; return env.render(); }
  if (what === 'add-cancel') { adding = false; addText = ''; return env.render(); }
  try {
    if (what === 'place') {
      rowBusy = `${appId}|${b.dataset.host}`; env.render();
      await invoke('set_trust', { appId, host: b.dataset.host, enabled: b.dataset.to === 'true' });
      rowBusy = '';
      await load(appId);
    }
    if (what === 'launcher') { const r = await invoke('app_launcher', { appId, reveal: true }); env.ctx.toast('In Finder: ' + tidy(r.path)); }
    if (what === 'workspace') await invoke('app_open_workspace', { appId });
    if (what === 'remove') { if (confirm('Stop protecting this app? Its files stay where they are.')) { env.ctx.setStatus(await invoke('app_remove', { appId })); env.go('overview'); } }
  } catch (e) { rowBusy = ''; error = errText(e); env.render(); }
}

export function leave() { shownId = null; box.leave(); }

export function onEvent({ type, data: ev }) {
  if (!shownId) return;
  if (type === 'feed' && ev.appId !== shownId) return;
  if (type !== 'feed' && type !== 'ask_resolved') return;
  // The agent is at work: what it has changed in its folders may have changed too.
  if (type === 'feed') projects.refresh(shownId, 3000);
  clearTimeout(reloadSoon);
  reloadSoon = setTimeout(() => shownId && load(shownId), 600);
}
