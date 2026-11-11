// Small shared helpers. No DOM framework: template strings, escaped, plus event delegation.

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ESC[c]);

/** Delegate `type` events from `root` to descendants matching `sel`. */
export function on(root, type, sel, fn) {
  root.addEventListener(type, (ev) => {
    const el = ev.target.closest(sel);
    if (el && root.contains(el)) fn(ev, el);
  });
}

/** Replace innerHTML only when it changed, so animations survive re-renders.
    Keyboard focus is carried across by matching `data-key`. */
export function patch(el, html) {
  if (el.__html === html) return false;
  const active = document.activeElement;
  const key = active && el.contains(active) ? active.getAttribute('data-key') : null;
  el.__html = html;
  el.innerHTML = html;
  if (key) {
    const next = el.querySelector(`[data-key="${key}"]`);
    if (next) next.focus({ preventScroll: true });
  }
  return true;
}

export const inApp = () => typeof window !== 'undefined' && !!window.__TAURI__;

/** Page boot: theme override from ?theme=, and a class the CSS can key on. */
export function initPage() {
  const root = document.documentElement;
  const theme = new URLSearchParams(location.search).get('theme');
  if (theme === 'light' || theme === 'dark') root.dataset.theme = theme;
  root.classList.add(inApp() ? 'in-app' : 'in-browser');
  if (window.parent !== window) root.classList.add('embedded');
  // Browser previews only: borrow the Fredoka display face from Google Fonts. The app itself never
  // contacts Google; it uses the Mac's rounded system font until Fredoka is bundled.
  if (!inApp() && !document.querySelector('link[data-preview-font]')) {
    const l = document.createElement('link');
    l.rel = 'stylesheet'; l.dataset.previewFont = '';
    l.href = 'https://fonts.googleapis.com/css2?family=Fredoka:wght@400..700&display=swap';
    document.head.appendChild(l);
  }
}

export function errText(e) {
  if (typeof e === 'string') return e;
  return (e && e.message) || 'Something went wrong. Please try again.';
}

// ---------- numbers ----------

const nf = new Intl.NumberFormat();
export const fmtCount = (n) => nf.format(n || 0);

export function fmtBytes(n) {
  if (n == null) return '';
  if (n < 1000) return n + (n === 1 ? ' byte' : ' bytes');
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n, i = -1;
  do { v /= 1000; i++; } while (v >= 999.5 && i < units.length - 1);
  return (v >= 100 ? Math.round(v) : v >= 10 ? Math.round(v * 10) / 10 : Math.round(v * 100) / 100) + ' ' + units[i];
}

export function fmtDuration(ms) {
  if (ms == null) return '';
  if (ms < 1000) return ms + ' ms';
  if (ms < 60000) return (Math.round(ms / 100) / 10) + ' s';
  return Math.round(ms / 60000) + ' min';
}

// ---------- time ----------

const DAY = 86400000;
const startOfDay = (t) => { const d = new Date(t); d.setHours(0, 0, 0, 0); return d.getTime(); };
const clock = (t) => new Date(t).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

export const isToday = (iso) => startOfDay(iso) === startOfDay(Date.now());

/** Compact relative time for one-line rows: "now", "4m", "2h", "Yesterday", "Mon", "9 Nov". */
export function relTime(iso, now = Date.now()) {
  const t = new Date(iso).getTime(), diff = now - t;
  if (diff < 45000) return 'now';
  if (diff < 3600000) return Math.max(1, Math.round(diff / 60000)) + 'm';
  const days = Math.round((startOfDay(now) - startOfDay(t)) / DAY);
  if (days === 0) return Math.round(diff / 3600000) + 'h';
  if (days === 1) return 'Yesterday';
  if (days < 7) return new Date(t).toLocaleDateString([], { weekday: 'short' });
  return new Date(t).toLocaleDateString([], { day: 'numeric', month: 'short' });
}

/** Feed time: relative when fresh, a clock time today, day + time before that. */
export function feedTime(iso, now = Date.now()) {
  const t = new Date(iso).getTime(), diff = now - t;
  if (diff < 45000) return 'Just now';
  if (diff < 3600000) return Math.max(1, Math.round(diff / 60000)) + ' min ago';
  const days = Math.round((startOfDay(now) - startOfDay(t)) / DAY);
  if (days === 0) return clock(t);
  if (days === 1) return 'Yesterday ' + clock(t);
  if (days < 7) return new Date(t).toLocaleDateString([], { weekday: 'short' }) + ' ' + clock(t);
  return new Date(t).toLocaleDateString([], { day: 'numeric', month: 'short' });
}

export const fullTime = (iso) => new Date(iso).toLocaleString([], { dateStyle: 'medium', timeStyle: 'medium' });

// ---------- shared vocabulary ----------

export const VERB = { allowed: 'Allowed', blocked: 'Blocked', asked: 'Asked', seen: 'Seen' };
export const verbChip = (verb) => `<span class="verb v-${esc(verb)}">${VERB[verb] || esc(verb)}</span>`;

export const MODE = { private: 'Private', watch: 'Watching', off: 'Off' };
export const dot = (indicator) => `<span class="dot d-${esc(indicator || 'off')}" aria-hidden="true"></span>`;

export const trustKey = (appId, host) => appId + '|' + host;

/** Accepts example.com or *.example.com; tolerates a pasted address. Returns the clean name or null. */
export function cleanHost(input) {
  let h = String(input || '').trim().toLowerCase();
  h = h.replace(/^[a-z]+:\/\//, '').replace(/[/?#].*$/, '').replace(/:\d+$/, '').replace(/\.$/, '');
  return /^(\*\.)?([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/.test(h) ? h : null;
}

// ---------- prompt countdown ----------

export const secondsLeft = (ask) => Math.max(0, Math.ceil((new Date(ask.expiresAt).getTime() - Date.now()) / 1000));

/** Drain a bar from the time left on `ask` down to zero. Compositor-driven; steps when motion is reduced. */
export function drain(bar, ask) {
  if (!bar || !bar.animate) return;
  const end = new Date(ask.expiresAt).getTime();
  const total = Math.max(1, end - new Date(ask.createdAt).getTime());
  const left = Math.max(0, end - Date.now());
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  bar.getAnimations().forEach((a) => a.cancel());
  bar.animate(
    [{ transform: `scaleX(${Math.min(1, left / total)})` }, { transform: 'scaleX(0)' }],
    { duration: left, easing: reduced ? `steps(${Math.max(1, Math.ceil(left / 1000))}, end)` : 'linear', fill: 'forwards' },
  );
}
