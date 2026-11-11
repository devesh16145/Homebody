// Seed data and pure text builders for the in-page mock. Browser development only.

export const SCENARIOS = ['setup', 'private', 'asking', 'watching', 'off', 'fence-down', 'empty', 'lockdown'];

/** host -> [label, purpose] */
export const CATALOG = {
  'api.anthropic.com': ['Anthropic API', 'Your AI provider'],
  'registry.ollama.ai': ['Ollama model library', 'Model download'],
  'ollama.com': ['Ollama website', 'Update check and model recommendations'],
  'statsig.anthropic.com': ['Statsig', 'Usage statistics'],
  'sentry.io': ['Sentry', 'Crash reports'],
  'registry.npmjs.org': ['npm registry', 'Package downloads'],
  'github.com': ['GitHub', 'Code hosting'],
  'pypi.org': ['Python package index', 'Package downloads'],
  'huggingface.co': ['Hugging Face', 'Model download'],
};

const APPS = [
  { id: 'ollama', name: 'Ollama', kind: 'model', icon: 'ollama', workspace: null },
  { id: 'claude-code', name: 'Claude Code', kind: 'agent', icon: 'claude', workspace: '~/Private AI/Claude Code' },
];

// Recognised but not installed; Aider shows the "installed where protected apps cannot go" case.
const MORE = [
  { id: 'codex', name: 'Codex CLI' }, { id: 'gemini', name: 'Gemini CLI' }, { id: 'opencode', name: 'opencode' },
  { id: 'aider', name: 'Aider', problem: 'Aider is installed inside your home folder (~/.local/bin/aider), where protected apps cannot go. Install it for the whole computer instead, for example with Homebrew or "npm install -g".' },
];

const PRESETS = [
  ['ollama', 'registry.ollama.ai', true, 'preset', true],
  ['claude-code', 'api.anthropic.com', true, 'preset', true],
  ['claude-code', 'statsig.anthropic.com', false, 'preset', false],
  ['claude-code', 'sentry.io', false, 'preset', false],
  ['claude-code', 'github.com', true, 'you', false],
];

const MIN = 60000, HOUR = 3600000, DAY = 86400000;
const iso = (t) => new Date(t).toISOString();

export const describe = (host) => CATALOG[host] || [host, ''];

export function trustEntry(appId, host, enabled, source = 'you', recommended = false) {
  const [label, purpose] = describe(host);
  return { appId, host, label, purpose, enabled, source, recommended };
}

/** One plain sentence for a FeedEvent. `pending` is the not-yet-answered state of an asked event. */
export function explain(reason, app, label, pending = false) {
  if (pending) return `${app} wants to reach ${label}. Waiting for your answer.`;
  switch (reason) {
    case 'trusted': return `${label} is on your Trusted list for ${app}, so it was allowed.`;
    case 'not_trusted': return `${app} tried to reach ${label}. It is not on your Trusted list, so it was blocked and nothing was sent.`;
    case 'you_allowed_once': return `You allowed this once. ${app} can reach ${label} for five minutes.`;
    case 'you_always_allow': return `You chose Always allow, so ${label} is now Trusted for ${app}.`;
    case 'you_blocked': return `You blocked ${label} for ${app}. Nothing was sent.`;
    case 'no_answer': return 'Nobody answered within 30 seconds, so it was blocked. Nothing was sent.';
    case 'watching': return 'Watch mode is on, so this was allowed and written down.';
    case 'protection_off': return 'Protection is off, so this was allowed and written down.';
    case 'lockdown': return 'Everything is blocked right now, so this was stopped. Nothing was sent.';
    case 'malformed': return `${app} tried to reach a bare number address instead of a named place. That is always blocked.`;
    case 'local_address': return 'This name points back to this computer or your home network, so it was blocked.';
    case 'around_gate': return `curl, started by ${app}, tried to connect straight to the internet on port 443, without going through the gate. The fence refused it, so nothing left this computer.`;
    default: return '';
  }
}

function ev(apps, t, appId, host, verb, reason, out = null, inn = null, ms = null) {
  const app = apps.find((a) => a.id === appId);
  const [label, purpose] = reason === 'around_gate' ? ['Direct connection', 'Went around the gate'] : describe(host);
  return {
    id: 0, ts: iso(t), appId, appName: app.name, appIcon: app.icon, host, port: 443, label, purpose,
    verb, reason, explanation: explain(reason, app.name, label), open: false,
    bytesOut: out, bytesIn: inn, durationMs: ms, trusted: false,
  };
}

/** Two weeks of believable history, oldest first. */
function history(apps, now) {
  const A = 'allowed', B = 'blocked', cc = 'claude-code', out = [];
  const add = (...a) => out.push(ev(apps, ...a));
  for (let d = 13; d >= 1; d--) {
    const base = now - d * DAY;
    add(base - 5 * HOUR, cc, 'api.anthropic.com', A, 'trusted', 14200 + d * 911, 88000 + d * 4099, 2400 + d * 37);
    add(base - 4.6 * HOUR, cc, 'statsig.anthropic.com', B, 'not_trusted');
    add(base - 2 * HOUR, cc, 'api.anthropic.com', A, 'trusted', 20100 + d * 613, 131000 + d * 2203, 3100 + d * 53);
    if (d % 4 === 0) add(base - 1.5 * HOUR, 'ollama', 'registry.ollama.ai', A, 'trusted', 3900, 2100000000 + d * 7e6, 52000);
    if (d % 3 === 0) add(base - 1.2 * HOUR, cc, 'sentry.io', B, 'not_trusted');
    if (d % 5 === 0) add(base - 1 * HOUR, cc, 'github.com', A, 'trusted', 5200, 284000, 900);
    if (d === 6) add(base - 0.8 * HOUR, 'ollama', 'nas.local', B, 'local_address');
    if (d === 3) add(base - 0.5 * HOUR, cc, '140.82.112.3', B, 'malformed');
    if (d === 2) add(base - 0.4 * HOUR, 'ollama', 'ollama.com', B, 'you_blocked');
  }
  add(now - 3 * HOUR, 'ollama', 'ollama.com', B, 'no_answer');
  add(now - 130 * MIN, cc, 'api.anthropic.com', A, 'trusted', 16400, 102000, 2700);
  out[out.length - 1].requests = [{ what: 'POST /v1/messages', times: 3 }, { what: 'POST /api/event_logging/v2/batch', times: 1 }, { what: 'GET /api/claude_cli/bootstrap', times: 1 }];
  add(now - 100 * MIN, cc, 'github.com', A, 'trusted', 6100, 310000, 1100);
  add(now - 58 * MIN, cc, 'sentry.io', B, 'not_trusted');
  add(now - 44 * MIN, cc, '', B, 'around_gate');
  add(now - 31 * MIN, 'ollama', 'registry.ollama.ai', A, 'trusted', 4100, 1900000000, 48000);
  add(now - 14 * MIN, cc, 'api.anthropic.com', A, 'trusted', 22300, 140000, 3300);
  add(now - 9 * MIN, cc, 'statsig.anthropic.com', B, 'not_trusted');
  add(now - 2 * MIN, cc, 'api.anthropic.com', A, 'trusted', 18400, 96000, 3100);
  return out;
}

function recent(apps, now, verb, reason, rows) {
  return rows.map(([minAgo, appId, host, o, i, ms]) => ev(apps, now - minAgo * MIN, appId, host, verb, reason, o, i, ms));
}

/** Build the starting state for a scenario. */
export function seed(scenario) {
  const now = Date.now();
  const none = scenario === 'setup';
  const s = {
    mode: 'private', modeSince: iso(now - 14 * DAY - 3 * HOUR), privateSince: iso(now - 14 * DAY - 3 * HOUR),
    lockdown: false, setupDone: true, fenceDown: false, setupReasons: [], askOnStart: null,
    apps: APPS.map((a) => ({ ...a, installed: true, protected: true, running: !none, custom: false, problem: null }))
      .concat(MORE.map((a) => ({ ...a, kind: 'agent', icon: 'generic', workspace: null, installed: !!a.problem, protected: true, running: false, custom: false, problem: a.problem || null }))),
    trust: PRESETS.filter((p) => !none || p[3] === 'preset').map((p) => trustEntry(...p)),
    events: [], seq: 1240,
  };
  let list = none || scenario === 'empty' ? [] : history(s.apps, now);
  const cc = 'claude-code';

  if (scenario === 'setup') {
    Object.assign(s, { mode: 'off', modeSince: iso(now), privateSince: null, setupDone: false, seq: 0 });
  } else if (scenario === 'empty') {
    Object.assign(s, { modeSince: iso(now - 2 * MIN), privateSince: iso(now - 2 * MIN), seq: 0 });
    s.apps.forEach((a) => { a.running = false; });
    s.trust = s.trust.filter((t) => t.source === 'preset');
  } else if (scenario === 'asking') {
    s.askOnStart = { appId: 'ollama', host: 'ollama.com' };
  } else if (scenario === 'watching') {
    Object.assign(s, { mode: 'watch', modeSince: iso(now - 40 * MIN), privateSince: null });
    list = list.filter((e) => now - new Date(e.ts) > 40 * MIN).concat(recent(s.apps, now, 'seen', 'watching', [
      [36, cc, 'api.anthropic.com', 19200, 121000, 2900], [33, cc, 'statsig.anthropic.com', 2100, 400, 300],
      [21, 'ollama', 'ollama.com', 1800, 24000, 600], [12, cc, 'registry.npmjs.org', 3300, 870000, 1900],
      [6, cc, 'sentry.io', 5200, 300, 400], [1, cc, 'api.anthropic.com', 17100, 99000, 2600],
    ]));
  } else if (scenario === 'off') {
    Object.assign(s, { mode: 'off', modeSince: iso(now - 2 * HOUR), privateSince: null });
    list = list.filter((e) => now - new Date(e.ts) > 2 * HOUR).concat(recent(s.apps, now, 'seen', 'protection_off', [
      [95, cc, 'api.anthropic.com', 15100, 91000, 2500], [44, cc, 'statsig.anthropic.com', 2300, 400, 300],
      [8, 'ollama', 'ollama.com', 1700, 22000, 500],
    ]));
  } else if (scenario === 'fence-down') {
    Object.assign(s, { setupDone: false, fenceDown: true, setupReasons: ['Protection was not restored after the last restart.'] });
    s.apps.forEach((a) => { a.running = false; });
  } else if (scenario === 'lockdown') {
    s.lockdown = true;
    list = list.concat(recent(s.apps, now, 'blocked', 'lockdown', [[1.5, cc, 'api.anthropic.com'], [0.6, 'ollama', 'registry.ollama.ai']]));
  }

  const isOn = (appId, host) => s.trust.some((t) => t.appId === appId && t.host === host && t.enabled);
  list.forEach((e) => { e.id = ++s.seq; e.trusted = isOn(e.appId, e.host); });
  s.events = list.reverse();
  return s;
}

// ---------- text builders ----------

function hash(str, len) {
  let h = 2166136261, out = '';
  for (let i = 0; i < str.length; i++) { h ^= str.charCodeAt(i); h = Math.imul(h, 16777619) >>> 0; }
  while (out.length < len) { out += h.toString(16).padStart(8, '0'); h = Math.imul(h ^ (h >>> 13), 2654435761) >>> 0; }
  return out.slice(0, len);
}

export const PUBLIC_KEY = 'MCowBQYDK2VwAyEA7kq1mVh0x2Zb3cR9tYp4uQe8LwJn5sFa6DgHi0oXrTc=';

/** Raw log lines for Studio -> Logs, oldest first, newest last. */
export function receiptLines(events, limit = 200) {
  const rows = events.slice(0, limit).reverse();
  let prev = hash('genesis', 16);
  return rows.map((e) => {
    const r = {
      seq: e.id, ts: e.ts, app: e.appId, to: e.host + ':' + e.port, verb: e.verb, reason: e.reason,
      out: e.bytesOut, in: e.bytesIn, prev, sig: hash(e.id + e.verb + e.ts, 24),
    };
    prev = hash(JSON.stringify(r), 16);
    return JSON.stringify(r);
  });
}

/** Raw settings for Studio -> Advanced. The only place technical words appear. */
export function policyText(s) {
  const apps = {};
  s.apps.forEach((a) => {
    apps[a.id] = {
      protected: a.protected,
      sandbox: a.kind === 'agent' ? { workspace: a.workspace, network: 'proxy-only' } : { network: 'proxy-only' },
      egress: {
        default: 'deny',
        allowlist: s.trust.filter((t) => t.appId === a.id && t.enabled).map((t) => ({ host: t.host, ports: [443], tls: 'passthrough', source: t.source })),
        denylist: s.trust.filter((t) => t.appId === a.id && !t.enabled).map((t) => t.host),
        unknown: 'ask',
      },
    };
  });
  return JSON.stringify({
    version: 1,
    mode: s.mode === 'private' ? 'enforce' : s.mode === 'watch' ? 'observe' : 'disabled',
    lockdown: s.lockdown,
    ask: { timeoutSeconds: 30, allowOnceMinutes: 5, quietMinutes: 10 },
    firewall: { anchor: 'pf', state: s.setupDone ? 'loaded' : 'missing' },
    proxy: { listen: 'loopback', tls: 'passthrough (never decrypted)' },
    ledger: { sign: 'ed25519', keep: 'local-only' },
    apps,
  }, null, 2);
}

export function streakText(sinceIso, now = Date.now()) {
  const ms = now - new Date(sinceIso).getTime();
  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
  if (ms < MIN) return 'Just now';
  if (ms < HOUR) return plural(Math.floor(ms / MIN), 'minute');
  if (ms < DAY) return plural(Math.floor(ms / HOUR), 'hour');
  return plural(Math.floor(ms / DAY), 'day');
}

export const dayText = (isoTs) => new Date(isoTs).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });

export const joinNames = (names) => names.length < 2 ? (names[0] || 'your AI apps')
  : names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1];
