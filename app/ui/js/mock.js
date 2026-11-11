// In-page implementation of the IPC contract (app/docs/ipc.md) for browser development.
// Never loaded inside the real app: api.js only imports it when window.__TAURI__ is missing.
import {
  SCENARIOS, seed, describe, explain, trustEntry, receiptLines, policyText,
  streakText, dayText, joinNames, PUBLIC_KEY,
} from './mock-data.js';

const ASK_MS = 30000, ONCE_MS = 5 * 60000, QUIET_MS = 10 * 60000, BLOCK_FLASH_MS = 60000;
const iso = (t = Date.now()) => new Date(t).toISOString();
const isIp = (h) => /^\d{1,3}(\.\d{1,3}){3}$/.test(h) || h.includes(':');
const sameDay = (a, b) => new Date(a).toDateString() === new Date(b).toDateString();
const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;

function bytesText(n) {
  if (n < 1000) return plural(n, 'byte');
  const u = ['KB', 'MB', 'GB'];
  let v = n, i = -1;
  do { v /= 1000; i++; } while (v >= 999.5 && i < 2);
  return (v >= 10 ? Math.round(v) : Math.round(v * 10) / 10) + ' ' + u[i];
}

const mockProjects = [];
const mockInside = [{ appId: 'claude-code', host: 'api.anthropic.com', hostLabel: 'Anthropic API', path: '/api/event_logging', label: 'Event logging', purpose: 'Usage statistics', enabled: false, source: 'preset' }];
const mockApplied = new Set();
const local = (v) => JSON.parse(JSON.stringify(v));

export function createMock(scenarioName) {
  const scenario = SCENARIOS.includes(scenarioName) ? scenarioName : 'private';
  const s = seed(scenario);
  const subs = new Set(), hostSubs = new Set(), timers = new Set();
  const asks = new Map();   // askId -> { ask, eventIds, timer }
  const once = new Map();   // app|host -> allowed until (ms)
  const quiet = new Map();  // app|host -> { until, reason }
  const flags = { failSetup: false, checking: false };
  let chain = null, lastBlocked = 0, statusTimer = 0, askSeq = 0, liveTimer = 0, disposed = false;

  // ---------- plumbing ----------
  function later(fn, ms) {
    const id = setTimeout(() => { timers.delete(id); if (!disposed) fn(); }, ms);
    timers.add(id);
    return id;
  }
  const wait = (ms) => new Promise((res) => later(res, ms));
  function emit(type, data) {
    const json = JSON.stringify({ type, data });
    later(() => subs.forEach((fn) => { try { fn(JSON.parse(json)); } catch (e) { subs.delete(fn); } }), 0);
  }
  function host(type, data = {}) { hostSubs.forEach((fn) => { try { fn({ type, ...data }); } catch (e) { /* harness only */ } }); }
  function queueStatus() {
    if (statusTimer) return;
    statusTimer = later(() => { statusTimer = 0; emit('status', status()); }, 60);
  }

  // ---------- derived state ----------
  const fence = () => flags.checking ? 'checking' : s.setupDone ? 'up' : s.fenceDown ? 'down' : 'not_installed';
  const key = (appId, h) => appId + '|' + h;
  function findTrust(appId, h) {
    return s.trust.find((t) => t.appId === appId && t.host === h)
      || s.trust.find((t) => t.appId === appId && t.host.startsWith('*.') && h.endsWith(t.host.slice(1)));
  }
  function counters(list) {
    const c = { bytesOut: 0, bytesIn: 0, allowed: 0, blocked: 0, asked: 0 };
    list.forEach((e) => {
      c.bytesOut += e.bytesOut || 0; c.bytesIn += e.bytesIn || 0;
      if (e.verb === 'blocked') c.blocked++; else if (e.verb !== 'asked') c.allowed++;
      if (e.verb === 'asked' || /^you_|no_answer/.test(e.reason)) c.asked++;
    });
    return c;
  }
  function sentence(indicator, today, pending) {
    if (!s.setupDone) return s.fenceDown ? 'Protection needs your attention.' : 'Not protected yet. Finish setup to start.';
    if (s.lockdown) return 'Everything is blocked until you release it.';
    if (s.mode === 'off') return 'Off. Your AI apps can reach anything.';
    if (indicator === 'asking') return `${pending[0].appName} is asking to reach ${pending[0].host}.`;
    if (s.mode === 'watch') {
      const places = new Set(s.events.filter((e) => e.reason === 'watching' && e.ts >= s.modeSince).map((e) => e.host)).size;
      return `Watching. ${plural(places, 'place')} contacted so far.`;
    }
    if (indicator === 'blocked') return 'Private. An attempt was just blocked.';
    return today.bytesOut === 0 ? 'Private. 0 bytes left today.' : `Private. ${bytesText(today.bytesOut)} sent, only to places you trust.`;
  }
  function status() {
    const f = fence(), now = Date.now();
    const todays = s.events.filter((e) => sameDay(e.ts, now));
    const today = counters(todays);
    const pending = [...asks.values()].map((a) => a.ask);
    let indicator = 'private';
    if (!s.setupDone || f === 'down') indicator = 'attention';
    else if (s.lockdown) indicator = 'blocked';
    else if (s.mode === 'off') indicator = 'off';
    else if (pending.length) indicator = 'asking';
    else if (s.mode === 'watch') indicator = 'watching';
    else if (now - lastBlocked < BLOCK_FLASH_MS) indicator = 'blocked';
    return {
      version: '0.1.0', mode: s.mode, modeSince: s.modeSince,
      privateSince: s.mode === 'private' ? s.privateSince : null,
      lockdown: s.lockdown, fence: f, setupNeeded: !s.setupDone, setupReasons: s.setupReasons.slice(),
      indicator, sentence: sentence(indicator, today, pending), today,
      apps: s.apps.map((a) => ({
        id: a.id, name: a.name, kind: a.kind, icon: a.icon, installed: a.installed, protected: a.protected,
        fence: a.protected ? f : 'not_installed', running: a.running,
        canStart: a.installed && a.protected && f === 'up' && !a.problem, workspace: a.workspace,
        custom: !!a.custom, problem: a.problem || null,
        inspect: !!a.inspect, canInspect: a.kind === 'agent', secrets: a.secrets !== false,
        today: counters(todays.filter((e) => e.appId === a.id)),
      })),
      pending,
      localServers: !!s.localServers,
      ask: s.ask !== false,
      presetNotes: s.presetsSeen ? [] : ['llama.cpp, Cursor CLI and OpenClaw are recognised. Nothing is trusted for them until you say so.'],
      watch: s.mode === 'watch' ? { since: s.modeSince, until: s.watchUntil || null, places: watchItems(s.modeSince).length } : null,
      watchReview: s.mode !== 'watch' && s.watchReview && watchItems(s.watchReview.since, s.watchReview.until).length
        ? { since: s.watchReview.since, until: s.watchReview.until, places: watchItems(s.watchReview.since, s.watchReview.until).length } : null,
    };
  }
  function watchItems(since, until) {
    const out = [];
    for (const e of s.events) {
      if (e.reason !== 'watching' || e.ts < since || (until && e.ts > until) || findTrust(e.appId, e.host)) continue;
      const hit = out.find((i) => i.appId === e.appId && i.host === e.host);
      if (hit) { hit.times++; hit.bytesOut += e.bytesOut || 0; }
      else out.push({ appId: e.appId, appName: e.appName, appIcon: e.appIcon, host: e.host, label: e.label, purpose: e.purpose, times: 1, bytesOut: e.bytesOut || 0 });
    }
    return out.sort((a, b) => a.appName.localeCompare(b.appName) || b.times - a.times);
  }
  function proof() {
    const priv = s.mode === 'private' && s.setupDone;
    const since = priv ? s.privateSince : null;
    const span = since ? s.events.filter((e) => e.ts >= since) : [];
    const names = s.apps.filter((a) => a.protected && a.installed).map((a) => a.name);
    const untrusted = span.filter((e) => e.reason === 'you_allowed_once').reduce((n, e) => n + (e.bytesOut || 0), 0);
    const who = joinNames(names);
    const statement = !priv ? 'Private is not on, so there is nothing to prove right now.'
      : untrusted ? `Since ${dayText(since)}, ${bytesText(untrusted)} left this Mac from ${who} to places you allowed once. Everything else went only to places you trust.`
      : `Nothing left this Mac from ${who} to anywhere you have not trusted since ${dayText(since)}.`;
    return {
      mode: s.mode, privateSince: since,
      streakDays: since ? Math.floor((Date.now() - new Date(since)) / 86400000) : 0,
      streakText: since ? streakText(since) : '',
      appsProtected: names.length, appNames: names,
      bytesOutToday: s.events.filter((e) => sameDay(e.ts, Date.now()) && e.trusted && e.verb === 'allowed').reduce((n, e) => n + (e.bytesOut || 0), 0),
      untrustedBytesSincePrivate: untrusted,
      blockedTotal: span.filter((e) => e.verb === 'blocked').length,
      allowedTotal: span.filter((e) => e.verb === 'allowed').length,
      chain, publicKey: PUBLIC_KEY, statement,
      covers: `Every connection that ${who} tried to make from this Mac${since ? ' since ' + dayText(since) : ''}. Each one was allowed or blocked, written down at that moment, and signed by this Mac so the record cannot be quietly changed later.`,
      cannotSee: 'Anything sent by apps that are not protected here, such as a web browser or an AI app started outside this protection. Anything sent before Private was turned on. What was inside an allowed connection: the amount is counted, the contents are never read.',
    };
  }

  // ---------- traffic ----------
  function touch(e, verb, reason, pending = false) {
    e.verb = verb; e.reason = reason;
    e.explanation = explain(reason, e.appName, e.label, pending);
    const t = findTrust(e.appId, e.host);
    e.trusted = !!(t && t.enabled);
  }
  function flow(e, sim = {}) {
    e.open = true;
    const ms = sim.ms || 1200 + Math.round(Math.random() * 1800);
    later(() => {
      if (e.verb === 'blocked') return;
      e.open = false; e.durationMs = ms;
      e.bytesOut = sim.out ?? 2000 + Math.round(Math.random() * 24000);
      e.bytesIn = sim.in ?? 20000 + Math.round(Math.random() * 180000);
      emit('feed', e); queueStatus();
    }, ms);
  }
  function blockedNow() {
    lastBlocked = Date.now();
    later(queueStatus, BLOCK_FLASH_MS + 200);
  }
  /** A protected app tries to reach somewhere. Applies the contract's rules. */
  function attempt(appId, h, sim = {}) {
    const app = s.apps.find((a) => a.id === appId);
    if (!app) return null;
    const [label, purpose] = describe(h);
    const k = key(appId, h), t = findTrust(appId, h), q = quiet.get(k), now = Date.now();
    const e = {
      id: ++s.seq, ts: iso(), appId, appName: app.name, appIcon: app.icon, host: h, port: 443, label, purpose,
      verb: 'blocked', reason: 'not_trusted', explanation: '', open: false,
      bytesOut: null, bytesIn: null, durationMs: null, trusted: false,
    };
    s.events.unshift(e);
    let ask = false;
    if (isIp(h)) touch(e, 'blocked', 'malformed');
    else if (/\.(local|home|lan)$/.test(h) || h === 'localhost') touch(e, 'blocked', 'local_address');
    else if (s.lockdown) touch(e, 'blocked', 'lockdown');
    else if (s.mode === 'off') touch(e, 'seen', 'protection_off');
    else if (s.mode === 'watch') touch(e, 'seen', 'watching');
    else if (t && t.enabled) touch(e, 'allowed', 'trusted');
    else if ((once.get(k) || 0) > now) touch(e, 'allowed', 'you_allowed_once');
    else if (t) touch(e, 'blocked', t.source === 'you' ? 'you_blocked' : 'not_trusted');
    else if (q && q.until > now) touch(e, 'blocked', q.reason);
    else { ask = true; touch(e, 'asked', 'not_trusted', true); e.open = true; }

    if (ask) openAsk(e, app, purpose);
    else if (e.verb === 'blocked') blockedNow();
    else flow(e, sim);
    emit('feed', e); queueStatus();
    return e;
  }
  function openAsk(e, app, purpose) {
    const existing = [...asks.values()].find((a) => a.ask.appId === e.appId && a.ask.host === e.host);
    if (existing) {
      existing.ask.waiting++; existing.eventIds.push(e.id);
      emit('ask', existing.ask);
      return;
    }
    const now = Date.now(), id = 'ask-' + (++askSeq);
    const ask = {
      id, appId: e.appId, appName: app.name, appIcon: app.icon, host: e.host, port: e.port, label: e.label, purpose,
      title: `${app.name} wants to reach ${e.host}`,
      body: purpose ? `${purpose}. Allow?` : 'This place is not on your Trusted list. Allow?',
      createdAt: iso(now), expiresAt: iso(now + ASK_MS), waiting: 1,
    };
    asks.set(id, { ask, eventIds: [e.id], timer: later(() => resolveAsk(id, 'timeout'), ASK_MS) });
    emit('ask', ask);
    host('show_prompt');
  }
  function resolveAsk(id, choice) {
    const a = asks.get(id);
    if (!a) return false;
    clearTimeout(a.timer); timers.delete(a.timer); asks.delete(id);
    const { appId, host: h } = a.ask, k = key(appId, h), now = Date.now();
    if (choice === 'always') upsertTrust(appId, h, true);
    else if (choice === 'once') once.set(k, now + ONCE_MS);
    else quiet.set(k, { until: now + QUIET_MS, reason: choice === 'block' ? 'you_blocked' : 'no_answer' });
    const reason = { always: 'you_always_allow', once: 'you_allowed_once', block: 'you_blocked', timeout: 'no_answer' }[choice];
    a.eventIds.forEach((eid) => {
      const e = s.events.find((x) => x.id === eid);
      if (!e) return;
      const ok = choice === 'always' || choice === 'once';
      touch(e, ok ? 'allowed' : 'blocked', reason);
      e.open = false;
      if (ok) flow(e); else blockedNow();
      emit('feed', e);
    });
    emit('ask_resolved', { id, choice });
    queueStatus();
    return true;
  }
  function resolveAll(choice) { [...asks.keys()].forEach((id) => resolveAsk(id, choice)); }

  // ---------- trust ----------
  function upsertTrust(appId, h, enabled) {
    const t = s.trust.find((x) => x.appId === appId && x.host === h);
    if (t) t.enabled = enabled; else s.trust.push(trustEntry(appId, h, enabled));
    retag(appId);
  }
  function retag(appId) {
    s.events.forEach((e) => { if (e.appId === appId) { const t = findTrust(appId, e.host); e.trusted = !!(t && t.enabled); } });
  }
  const trustList = (appId) => s.trust.filter((t) => !appId || t.appId === appId);
  function needApp(appId) {
    const a = s.apps.find((x) => x.id === appId);
    if (!a) throw 'That app is not known on this computer.';
    return a;
  }

  // ---------- commands ----------
  const commands = {
    async get_status() { return status(); },
    async get_watch_review() {
      if (s.mode === 'watch') return { since: s.modeSince, until: s.watchUntil || null, active: true, items: watchItems(s.modeSince) };
      if (!s.watchReview) return null;
      return { since: s.watchReview.since, until: s.watchReview.until, active: false, items: watchItems(s.watchReview.since, s.watchReview.until) };
    },
    async finish_watch_review({ trust = [] } = {}) {
      trust.forEach((t) => upsertTrust(t.appId, t.host, true));
      s.watchReview = null;
      queueStatus();
      return status();
    },
    async set_mode({ mode, minutes } = {}) {
      if (!['private', 'watch', 'off'].includes(mode)) throw 'That mode does not exist.';
      if (!s.setupDone && mode !== 'off') throw 'Finish setup first, then you can turn this on.';
      if (mode === 'watch') s.watchUntil = minutes === 0 ? null : iso(Date.now() + (minutes || 60) * 60000);
      if (mode !== s.mode) {
        if (s.mode === 'watch') s.watchReview = { since: s.modeSince, until: iso() };
        if (mode === 'watch') s.watchReview = null;
        s.mode = mode; s.modeSince = iso();
        s.privateSince = mode === 'private' ? s.modeSince : null;
        if (mode !== 'private') resolveAll('once');
      }
      queueStatus();
      return status();
    },
    async set_lockdown({ on } = {}) {
      s.lockdown = !!on;
      if (s.lockdown) resolveAll('block');
      queueStatus();
      return status();
    },
    async get_feed({ limit = 200, beforeId, appId, verb } = {}) {
      return s.events.filter((e) => (!beforeId || e.id < beforeId) && (!appId || e.appId === appId) && (!verb || e.verb === verb)).slice(0, limit);
    },
    async decide({ askId, choice } = {}) {
      if (!['once', 'always', 'block'].includes(choice)) throw 'That answer is not one of the three choices.';
      if (!resolveAsk(askId, choice)) throw 'That question was already answered.';
      return null;
    },
    async get_trust({ appId } = {}) { return trustList(appId); },
    async set_trust({ appId, host: h, enabled } = {}) {
      needApp(appId);
      if (!h) throw 'A destination name is needed.';
      upsertTrust(appId, h, !!enabled);
      const pending = [...asks.values()].find((a) => a.ask.appId === appId && a.ask.host === h);
      if (pending) resolveAsk(pending.ask.id, enabled ? 'always' : 'block');
      queueStatus();
      return trustList();
    },
    async remove_trust({ appId, host: h } = {}) {
      const i = s.trust.findIndex((t) => t.appId === appId && t.host === h);
      if (i < 0) throw 'That destination is not on the list.';
      if (s.trust[i].source !== 'you') throw 'Built-in destinations can be switched off, but not removed.';
      s.trust.splice(i, 1);
      retag(appId);
      queueStatus();
      return trustList();
    },
    async get_proof() { return proof(); },
    async verify_chain() {
      await wait(900);
      chain = { ok: true, count: s.events.length ? s.seq : 1, checkedAt: iso(), error: null };
      return chain;
    },
    async export_log() {
      await wait(500);
      const path = `~/Downloads/Homebody log ${iso().slice(0, 16).replace('T', ' ').replace(':', '.')}.jsonl`;
      host('note', { text: 'Wrote and revealed ' + path });
      return { path };
    },
    async export_proof() {
      await wait(500);
      const stamp = iso().slice(0, 16).replace(/[-:]/g, '').replace('T', '-');
      const path = `~/Downloads/private-proof-${stamp}.json`;
      host('note', { text: 'Wrote and revealed ' + path });
      return { path };
    },
    async get_receipts({ limit = 200 } = {}) {
      const lines = receiptLines(s.events, limit);
      return lines.length ? lines : [JSON.stringify({ seq: 1, ts: s.modeSince, event: 'protection_on', sig: '5f1c0e9a77b2d4c3a1e06b8d' })];
    },
    async get_policy_text() { return policyText(s); },
    async pick_folder() { return '/Users/you/Documents/Projects/recipe-site'; },
    async get_projects() { return local(mockProjects); },
    async project_import({ path }) {
      const name = String(path).split('/').filter(Boolean).pop() || 'project';
      const p = { name, source: path, copy: '/Users/Shared/SandboxAI/claude-code/' + name, importedAt: new Date().toISOString(), sourceExists: true, copyExists: true };
      mockProjects.push(p); return { ...local(p), suggested: [{ host: 'registry.npmjs.org', label: 'npm registry', purpose: 'Package downloads' }, { host: 'pypi.org', label: 'Python Package Index', purpose: 'Package downloads' }] };
    },
    async project_changes({ name }) {
      const applied = mockApplied.has(name);
      const items = applied ? [{ path: 'README.md', kind: 'modified', conflict: true, link: null }] : [
        { path: 'README.md', kind: 'modified', conflict: true, link: null },
        { path: 'src/app.py', kind: 'modified', conflict: false, link: null },
        { path: 'src/recipes/soup.py', kind: 'added', conflict: false, link: null },
        { path: 'src/old_helpers.py', kind: 'deleted', conflict: false, link: null },
        { path: 'config', kind: 'added', conflict: false, link: '/etc/hosts' }];
      return { total: items.length, conflicts: 1, items, truncated: false };
    },
    async project_apply({ name, overwrite }) { const first = !mockApplied.has(name); mockApplied.add(name); return { applied: (first ? 4 : 0) + (overwrite ? 1 : 0), skipped: overwrite ? [] : ['README.md'] }; },
    async project_remove({ name }) { const i = mockProjects.findIndex((p) => p.name === name); if (i >= 0) mockProjects.splice(i, 1); return null; },
    async get_setup_script() { return '#!/bin/bash\n# Homebody one-time setup (mock).\n# The real app shows the exact script here.\n'; },
    async run_setup() {
      await wait(1600);
      if (flags.failSetup) return { ok: false, message: 'Setup was cancelled before it finished. Nothing was changed.' };
      s.localServers = true;
      if (!s.setupDone || s.fenceDown) {
        Object.assign(s, { setupDone: true, fenceDown: false, setupReasons: [], mode: 'private', modeSince: iso() });
        s.privateSince = s.modeSince;
      }
      queueStatus();
      return { ok: true, message: 'Protection is on.' };
    },
    async remove_setup() {
      await wait(1200);
      if (flags.failSetup) return { ok: false, message: 'Removal was cancelled before it finished. Protection is still on.' };
      resolveAll('block');
      Object.assign(s, { setupDone: false, fenceDown: false, setupReasons: [], mode: 'off', modeSince: iso(), privateSince: null, lockdown: false });
      s.apps.forEach((a) => { a.running = false; });
      queueStatus();
      return { ok: true, message: 'Protection was removed from this computer.' };
    },
    async check_fence() {
      flags.checking = true; emit('status', status());
      await wait(700);
      flags.checking = false; queueStatus();
      return status();
    },
    async pick_program() { return '/Applications/LM Studio.app'; },
    async discover_apps() {
      const all = [
        { name: 'LM Studio', path: '/Applications/LM Studio.app', canProtect: true, asks: true, note: 'Everything it sends will go through the gate, so you are asked.' },
        { name: 'Cursor', path: '/Applications/Cursor.app', canProtect: true, asks: true, note: 'Everything it sends will go through the gate, so you are asked.' },
        { name: 'ChatGPT', path: '/Applications/ChatGPT.app', canProtect: true, asks: false, note: 'Can only be run cut off from the internet: this kind of app cannot be pointed at the gate yet.' },
        { name: 'Draw Things', path: '/Applications/Draw Things.app', canProtect: false, asks: false, note: "Draw Things already runs in Apple's own sandbox, and macOS does not allow a second one around it. Apps like this need the Network Extension version of Homebody." },
      ];
      return all.filter((f) => !s.apps.some((a) => a.name === f.name));
    },
    async app_launcher({ appId } = {}) { const a = needApp(appId); return { path: `~/Applications/Homebody/${a.name} (protected).app` }; },
    async app_open({ appId } = {}) { needApp(appId).running = true; queueStatus(); return null; },
    async get_app_icons() { return {}; },   // the preview uses .local-icons instead (icons.js)
    async get_install() { return s.installed ? { path: '/Applications/Homebody.app', installed: true, canInstall: false } : { path: '/Volumes/Homebody/Homebody.app', installed: false, canInstall: true }; },
    async install_and_restart() { await wait(900); s.installed = true; host('note', { text: 'Copied to /Applications and restarted' }); return null; },
    async get_login_item() { return { on: !!s.loginItem }; },
    async set_login_item({ on } = {}) { s.loginItem = !!on; return { on: s.loginItem }; },
    async app_add({ name, program } = {}) {
      await wait(400);
      const clean = String(name || '').replace(/[^\p{L}\p{N} ._+-]/gu, '').trim();
      if (!clean) throw 'Give the app a short name using letters and numbers.';
      if (!String(program || '').trim()) throw 'Choose the program to run.';
      if (s.apps.some((a) => a.name.toLowerCase() === clean.toLowerCase())) throw `There is already an app called ${clean}.`;
      const id = 'my-' + clean.toLowerCase().replace(/[^a-z0-9]+/g, '-');
      const win = String(program).trim().endsWith('.app');
      if (win && /textedit|notes|safari/i.test(program)) throw `${clean} already runs in Apple's own sandbox, and macOS does not allow a second one around it. Apps like this need the Network Extension version of Homebody.`;
      s.apps.push({ id, name: clean, kind: win ? 'window' : 'agent', icon: 'generic', workspace: win ? null : '/Users/Shared/SandboxAI/' + id, installed: true, protected: true, running: false, custom: true, problem: null });
      queueStatus();
      return status();
    },
    async app_remove({ appId } = {}) {
      const a = needApp(appId);
      if (!a.custom) throw `${a.name} is built in. Switch its protection off instead.`;
      s.apps.splice(s.apps.indexOf(a), 1);
      queueStatus();
      return status();
    },
    async get_inside_rules() { return local(mockInside); },
    async set_inside_rule({ appId, host, path, enabled } = {}) {
      const r = mockInside.find((x) => x.appId === appId && x.host === host && x.path === path);
      if (r) r.enabled = !!enabled; else mockInside.push({ appId, host, hostLabel: 'Anthropic API', path, label: '', purpose: '', enabled: !!enabled, source: 'you' });
      s.events.forEach((e) => (e.requests || []).forEach((q) => { if (e.appId === appId && e.host === host && q.path.startsWith(path)) q.allowed = !!enabled; }));
      return local(mockInside);
    },
    async remove_inside_rule({ appId, host, path } = {}) {
      const i = mockInside.findIndex((x) => x.appId === appId && x.host === host && x.path === path && x.source === 'you');
      if (i >= 0) mockInside.splice(i, 1);
      return local(mockInside);
    },
    async set_ask({ on } = {}) { s.ask = !!on; queueStatus(); return status(); },
    async ack_presets() { s.presetsSeen = true; queueStatus(); return status(); },
    async set_limit({ appId, host, mb } = {}) { const t = s.trust.find((x) => x.appId === appId && x.host === host); if (t) { t.limitMb = mb || null; t.sentMb = mb ? 0.4 : null; } return trustList(); },
    async get_private_folders() { return { folders: (s.privateFolders || []).map((path) => ({ path, exists: true })), files: (s.privateFolders || []).length ? 212 : null, lines: (s.privateFolders || []).length ? 5310 : null }; },
    async add_private_folder({ path } = {}) { await wait(700); s.privateFolders = (s.privateFolders || []).concat(path.replace('recipe-site', 'Taxes')); return this.get_private_folders(); },
    async remove_private_folder({ path } = {}) { s.privateFolders = (s.privateFolders || []).filter((f) => f !== path); return this.get_private_folders(); },
    async get_summary() {
      return { days: 7, since: iso(Date.now() - 7 * 864e5), apps: [
        { appId: 'claude-code', appName: 'Claude Code', allowed: 41, blocked: 12, bytesOut: 2140000, aroundGate: 2, secretsStopped: 1, addressesRefused: 6,
          places: [{ name: 'Anthropic API', times: 38, bytesOut: 2100000 }, { name: 'GitHub', times: 3, bytesOut: 40000 }], blockedPlaces: [{ name: 'Statsig', times: 7 }, { name: 'Sentry', times: 3 }] },
        { appId: 'ollama', appName: 'Ollama', allowed: 2, blocked: 5, bytesOut: 8200, aroundGate: 0, secretsStopped: 0, addressesRefused: 0,
          places: [{ name: 'Ollama model library', times: 2, bytesOut: 8200 }], blockedPlaces: [{ name: 'Ollama website', times: 5 }] }] };
    },
    async timestamp_log() { await wait(900); return { records: s.seq || 1240, time: 'Nov 11 09:14:02 2026 GMT', service: 'http://timestamp.digicert.com', file: '~/Library/…/record-1240.tsr' }; },
    async check_update() { await wait(700); host('note', { text: 'Asked where Homebody is published' }); return { current: '0.1.0', latest: null, newer: false, url: null, message: 'No version has been published yet.' }; },
    async app_set_secrets({ appId, on } = {}) { needApp(appId).secrets = !!on; queueStatus(); return status(); },
    async allow_secret({ appId, fingerprint, allowed } = {}) {
      s.events.forEach((e) => { if (e.secret && e.appId === appId && e.secret.fingerprint === fingerprint) e.secret.allowed = !!allowed; });
      return null;
    },
    async app_set_inspect({ appId, on } = {}) {
      const a = needApp(appId);
      if (a.kind !== 'agent') throw `${a.name} takes its trusted certificates from your Mac, which Homebody does not change. Its connections cannot be looked inside yet.`;
      a.inspect = !!on; queueStatus(); return status();
    },
    async app_set_protected({ appId, protected: on } = {}) {
      const a = needApp(appId);
      a.protected = !!on;
      if (!a.protected) a.running = false;
      queueStatus();
      return status();
    },
    async app_start({ appId } = {}) {
      const a = needApp(appId);
      if (!(a.installed && a.protected && fence() === 'up')) throw `${a.name} cannot start until protection is set up for it.`;
      await wait(350);
      a.running = true;
      queueStatus();
      return status();
    },
    async app_stop({ appId } = {}) {
      const a = needApp(appId);
      await wait(200);
      a.running = false;
      queueStatus();
      return status();
    },
    async app_open_terminal({ appId } = {}) { host('note', { text: `Opened a protected terminal for ${needApp(appId).name}` }); return null; },
    async app_open_workspace({ appId } = {}) { host('note', { text: `Revealed ${needApp(appId).workspace} in Finder` }); return null; },
    async open_main({ tab } = {}) {
      host('open_main', { tab });
      if (tab) emit('navigate', { tab });
      return null;
    },
    async hide_window(_args, ctx) { host('hide_window', { window: ctx.window }); return null; },
    async quit() { host('note', { text: 'The app would quit now' }); return null; },
  };

  // ---------- harness-only controls (dev.html) ----------
  const ASKABLE = [['ollama', 'ollama.com'], ['claude-code', 'registry.npmjs.org'], ['claude-code', 'pypi.org'], ['ollama', 'huggingface.co']];
  function simulate(kind) {
    switch (kind) {
      case 'ask': {
        const now = Date.now();
        const free = ASKABLE.find(([a, h]) => !findTrust(a, h) && !((once.get(key(a, h)) || 0) > now) && !((quiet.get(key(a, h)) || {}).until > now)
          && ![...asks.values()].some((x) => x.ask.appId === a && x.ask.host === h));
        return attempt(...(free || ['claude-code', `files-${1 + (s.seq % 9)}.example.net`]));
      }
      case 'allowed': return attempt('claude-code', 'api.anthropic.com', { ms: 2600 });
      case 'blocked': return attempt('claude-code', 'statsig.anthropic.com');
      case 'download': return attempt('ollama', 'registry.ollama.ai', { ms: 4200, out: 4300, in: 2019000000 });
      case 'odd': return attempt('claude-code', '140.82.112.3');
      default: {
        const pick = ['allowed', 'allowed', 'allowed', 'blocked', 'download', 'blocked'];
        return simulate(pick[Math.floor(Math.random() * pick.length)]);
      }
    }
  }
  function setLive(on) {
    clearInterval(liveTimer); liveTimer = 0;
    if (on) liveTimer = setInterval(() => simulate('any'), 2600);
  }

  if (s.askOnStart) later(() => attempt(s.askOnStart.appId, s.askOnStart.host), 0);

  return {
    scenario,
    /** Same shape as Tauri's invoke: resolves with data, rejects with a plain string. */
    async invoke(cmd, args, ctx = {}) {
      if (disposed) throw 'The app is not running.';
      const fn = commands[cmd];
      if (!fn) throw `Unknown command: ${cmd}`;
      await wait(25);
      const out = await fn(args || {}, ctx);
      return out == null ? null : JSON.parse(JSON.stringify(out));
    },
    subscribe(fn) { subs.add(fn); return () => subs.delete(fn); },
    onHost(fn) { hostSubs.add(fn); return () => hostSubs.delete(fn); },
    simulate, setLive,
    setFlag(name, value) { flags[name] = value; },
    dispose() { disposed = true; clearInterval(liveTimer); timers.forEach(clearTimeout); timers.clear(); subs.clear(); hostSubs.clear(); },
  };
}
