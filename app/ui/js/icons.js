// Inline SVG icons (16px grid, 1.5 stroke) and app monogram tiles. No brand logos.
import { esc } from './util.js';

const P = {
  gear: '<circle cx="8" cy="8" r="2.1"/><path d="M8 1.6v1.7M8 12.7v1.7M1.6 8h1.7M12.7 8h1.7M3.5 3.5l1.2 1.2M11.3 11.3l1.2 1.2M12.5 3.5l-1.2 1.2M4.7 11.3l-1.2 1.2"/>',
  check: '<path d="M3.2 8.4l3 3 6.6-6.8"/>',
  x: '<path d="M4 4l8 8M12 4l-8 8"/>',
  plus: '<path d="M8 3.2v9.6M3.2 8h9.6"/>',
  chevron: '<path d="M6 3.5L10.5 8 6 12.5"/>',
  alert: '<path d="M8 2.2l6.2 11H1.8z"/><path d="M8 6.6v3M8 11.6v.1"/>',
  info: '<circle cx="8" cy="8" r="6.2"/><path d="M8 7.4v3.6M8 5v.1"/>',
  box: '<path d="M2.4 5L8 2.2 13.6 5v6L8 13.8 2.4 11z"/><path d="M2.4 5L8 7.8 13.6 5M8 7.8v6"/>',
  lock: '<rect x="3.4" y="7" width="9.2" height="6.8" rx="1.8"/><path d="M5.4 7V5.2a2.6 2.6 0 015.2 0V7"/>',
  shield: '<path d="M8 1.8l5.2 2v3.9c0 3.1-2.1 5.4-5.2 6.5-3.1-1.1-5.2-3.4-5.2-6.5V3.8z"/>',
  share: '<path d="M8 10V2M5.2 4.6L8 1.8l2.8 2.8"/><path d="M4.6 7.2H3.4v6.6h9.2V7.2h-1.2"/>',
  terminal: '<rect x="1.8" y="2.8" width="12.4" height="10.4" rx="2"/><path d="M4.6 6.2L6.8 8l-2.2 1.8M8.6 10h2.8"/>',
  folder: '<path d="M1.8 4.6c0-.8.6-1.4 1.4-1.4h3l1.4 1.6h5.2c.8 0 1.4.6 1.4 1.4v5.4c0 .8-.6 1.4-1.4 1.4H3.2c-.8 0-1.4-.6-1.4-1.4z"/>',
  list: '<path d="M5.6 4.2h8M5.6 8h8M5.6 11.8h8M2.4 4.2h.1M2.4 8h.1M2.4 11.8h.1"/>',
  apps: '<rect x="2.2" y="2.2" width="4.8" height="4.8" rx="1.3"/><rect x="9" y="2.2" width="4.8" height="4.8" rx="1.3"/><rect x="2.2" y="9" width="4.8" height="4.8" rx="1.3"/><rect x="9" y="9" width="4.8" height="4.8" rx="1.3"/>',
  receipt: '<path d="M3.6 1.8h8.8v12.4l-2.2-1.4L8 14.2l-2.2-1.4-2.2 1.4z"/><path d="M6 5.4h4M6 8.2h4"/>',
  sliders: '<path d="M2 4.6h6.4M11.6 4.6H14M2 11.4h2.4M7.6 11.4H14"/><circle cx="10" cy="4.6" r="1.6"/><circle cx="6" cy="11.4" r="1.6"/>',
  play: '<path d="M5 3.2l7.2 4.8L5 12.8z"/>',
  stop: '<rect x="4" y="4" width="8" height="8" rx="1.6"/>',
  link: '<path d="M6.8 9.2a2.8 2.8 0 004 0l2-2a2.8 2.8 0 00-4-4l-.7.7"/><path d="M9.2 6.8a2.8 2.8 0 00-4 0l-2 2a2.8 2.8 0 004 4l.7-.7"/>',
  feed: '<path d="M2.4 4h11.2M2.4 8h11.2M2.4 12h7"/>',
  spark: '<circle cx="8" cy="8" r="2.2"/>',
};

export function icon(name, cls = '') {
  return `<svg class="ic ${cls}" viewBox="0 0 16 16" aria-hidden="true">${P[name] || ''}</svg>`;
}

// App icons. An app shows its own icon when the maker's Mac app gives us one (setAppIcons);
// otherwise a glass tile in the maker's colour with a short monogram. No brand logos are drawn here.
// [top colour, bottom colour, letters, dark letters?]
const TINTS = {
  'claude code': ['#e9946c', '#b4502c', 'Cc'],
  ollama: ['#ffffff', '#d5dbdf', 'O', true],
  'codex cli': ['#3a3d42', '#0d0e10', 'Cx'],
  'gemini cli': ['#7fb2ff', '#4b57e0', 'G'],
  aider: ['#46cf95', '#178555', 'Ai'],
  opencode: ['#59606a', '#1f2328', 'oc'],
  'llama.cpp': ['#ffbe5c', '#d86f17', 'll'],
  'cursor cli': ['#4a4a4e', '#121214', 'Cu'],
  openclaw: ['#f7776a', '#c0372b', 'Cl'],
  'lm studio': ['#9b8cff', '#5a47d6', 'LM'],
};
const SPARE = [['#8fd3e8', '#2f86a6'], ['#b9c6a0', '#5f7a3b'], ['#e7b7d0', '#a14d7d'], ['#c2b49c', '#7b6243'], ['#a9b6c2', '#4f6070']];
const REAL = new Map();   // lower-case app name -> image URL

/** Icons taken from the makers' own Mac apps, by app name. */
export function setAppIcons(map) { for (const [k, v] of Object.entries(map || {})) REAL.set(k.toLowerCase(), v); }

// In a browser preview, use icons copied from this Mac's own apps (app/ui/.local-icons, not in git).
// In the real app the core supplies them. Missing files fall back to the tile.
if (typeof window !== 'undefined' && !window.__TAURI__) {
  setAppIcons({ 'Claude Code': '.local-icons/claude.png', 'Codex CLI': '.local-icons/chatgpt.png', 'Cursor CLI': '.local-icons/cursor.png' });
}

/** An app's icon. size: 16 | 20 | 24 | 28 | 36 | 44 | 56 | 72 */
export function appTile(key, name = '', size = 20) {
  const n = String(name).trim().toLowerCase();
  let t = TINTS[n] || (key === 'claude' ? TINTS['claude code'] : key === 'ollama' ? TINTS.ollama : null);
  if (!t) { let h = 0; for (const c of n) h = (h * 31 + c.charCodeAt(0)) >>> 0; t = SPARE[h % SPARE.length]; }
  const letters = t[2] || (name ? name.trim().charAt(0).toUpperCase() : '');
  const glyph = letters ? esc(letters) : icon('spark');
  const real = REAL.get(n);
  const img = real ? `<img class="tile-img" src="${esc(real)}" alt="" onerror="this.parentNode.classList.remove('real');this.remove()">` : '';
  return `<span class="tile${real ? ' real' : ''}${t[3] ? ' ink-dark' : ''}${letters.length > 1 ? ' two' : ''}${size !== 20 ? ' s' + size : ''}" style="--t1:${t[0]};--t2:${t[1]}" aria-hidden="true"><span class="tile-g">${glyph}</span>${img}</span>`;
}

/* ---------- The Homebody mark: a cat peeking over the windowsill. ---------- */
// Geometry is shared with src-tauri/icons/make-icons.swift (64 box, y down, sill at y 40).

const CAT_HEAD = '<path d="M13 42A19 19 0 0 1 51 42Z"/><path d="M15.5 33.5L17.5 13.5Q18 11 20 12.5L31 23.5Z"/><path d="M48.5 33.5L46.5 13.5Q46 11 44 12.5L33 23.5Z"/>';
const catEyes = (dx = 0, dy = 0) => `<ellipse cx="${25 + dx}" cy="${32 + dy}" rx="2.6" ry="3.7"/><ellipse cx="${39 + dx}" cy="${32 + dy}" rx="2.6" ry="3.7"/>`;
let seq = 0;

/**
 * state: 'private' (peeking), 'ask' (amber, looks up at you), 'shut' (ducked down: only the ear
 * tips show, everything is blocked), 'off' (faded: not protected). live: blinks now and then.
 */
export function mark(size = 44, state = 'private', live = false, anim = '') {
  const k = 'hb' + (++seq);
  const [dx, dy] = state === 'ask' ? [1.8, -1.8] : [0, 0];
  const tone = state === 'off' ? 'var(--ink-3)' : 'var(--tide)';
  const head = state === 'ask' ? 'var(--ask)' : 'currentColor';
  const cat = `<g mask="url(#${k}m)" style="fill:${head}">${CAT_HEAD}</g>`;
  const cls = `mark mark-${state}${live ? ' mark-live' : ''}${anim ? ' mark-' + anim : ''}`;
  return `<svg class="${cls}" width="${size}" height="${size}" viewBox="0 0 64 64" aria-hidden="true" style="color:${tone}${state === 'off' ? ';opacity:.55' : ''}">
    <defs><mask id="${k}m"><rect x="-10" y="-10" width="90" height="90" fill="#fff"/><g fill="#000">${state === 'shut' ? '' : catEyes(dx, dy)}</g></mask>
      <clipPath id="${k}c"><rect width="64" height="40.5"/></clipPath></defs>
    <g clip-path="url(#${k}c)"><g class="hb-cat"${state === 'shut' ? ' transform="translate(0 15)"' : ''}>${cat}</g></g>
    <rect x="6" y="40" width="52" height="6" rx="3" fill="currentColor"/>
    ${state === 'shut' ? '' : '<circle class="hb-paw" cx="18" cy="41" r="3.8" fill="currentColor"/><circle class="hb-paw" cx="46" cy="41" r="3.8" fill="currentColor"/>'}
  </svg>`;
}

// ---------- the cat moves ----------
//
// Each place that shows the cat keeps the state it showed last. When the state changes the cat
// moves once: it peeks up from behind the sill, ducks down, or perks up with a question. Animation
// classes are only present for that one drawing, so redrawing the page never replays them.
// A movement stays in the drawing for its own length, so a quick redraw (several happen while a
// window opens) restarts it instead of cutting it off.
const lastShown = new Map();   // slot -> { state, move, at }
const MOVE_MS = 700;

/** Which movement to play in `slot` for `state`: 'peek', 'duck', 'perk', or ''. `fresh` forces a peek. */
export function markMove(slot, state, fresh = false) {
  const prev = lastShown.get(slot), now = Date.now();
  if (prev && prev.state === state && !fresh) return now - prev.at < MOVE_MS ? prev.move : '';
  let move = '';
  if (state === 'shut') move = prev && prev.state !== 'shut' ? 'duck' : '';
  else if (fresh || !prev || prev.state === 'shut') move = 'peek';
  else if (state === 'ask') move = 'perk';
  lastShown.set(slot, { state, move, at: now });
  return move;
}

/** True for a while after `key` is first seen: for things that should arrive with a little motion. */
const firstSeen = new Map();
export function isNew(key, ms = MOVE_MS) {
  const now = Date.now();
  if (!firstSeen.has(key)) firstSeen.set(key, now);
  return now - firstSeen.get(key) < ms;
}

/** Forget what a slot showed, so the next drawing peeks again (a window being shown, a page entered). */
export function markReset(slot) { lastShown.delete(slot); }

/** Live cats blink now and then: one quiet timer for the page, paused while the page is hidden. */
let blinkTimer = 0;
export function startBlinking() {
  if (blinkTimer || (window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches)) return;
  const tick = () => {
    blinkTimer = setTimeout(tick, 4200 + Math.random() * 4200);
    if (document.hidden) return;
    const eyes = document.querySelectorAll('svg.mark-live mask ellipse');
    eyes.forEach((e) => { e.dataset.ry = e.getAttribute('ry'); e.setAttribute('ry', '0.5'); });
    setTimeout(() => eyes.forEach((e) => e.setAttribute('ry', e.dataset.ry || '3.7')), 130);
  };
  blinkTimer = setTimeout(tick, 2500);
}

/** The Homebody app icon, as drawn for the Dock. */
export function appIcon(size = 128) {
  const k = 'hbi' + (++seq);
  return `<svg width="${size}" height="${size}" viewBox="0 0 100 100" aria-hidden="true" class="app-icon">
    <defs>
      <linearGradient id="${k}g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5ad6de"/><stop offset="1" stop-color="#077a86"/></linearGradient>
      <linearGradient id="${k}f" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#d9f3f5"/></linearGradient>
      <linearGradient id="${k}s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".28"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient>
      <clipPath id="${k}t"><rect x="4" y="4" width="92" height="92" rx="22"/></clipPath>
      <clipPath id="${k}c"><rect width="64" height="40.5"/></clipPath>
    </defs>
    <g clip-path="url(#${k}t)">
      <rect x="4" y="4" width="92" height="96" fill="url(#${k}g)"/>
      <g transform="translate(9.6 6.4) scale(1.3)"><g clip-path="url(#${k}c)"><g fill="url(#${k}f)">${CAT_HEAD}</g><g fill="#0b1620">${catEyes()}</g></g></g>
      <rect x="4" y="58.4" width="92" height="42" fill="#0b1620" fill-opacity=".86"/>
      <rect x="4" y="58.4" width="92" height="2" fill="#fff" fill-opacity=".25"/>
      <circle cx="33" cy="59.7" r="5" fill="url(#${k}f)"/><circle cx="69.4" cy="59.7" r="5" fill="url(#${k}f)"/>
      <rect x="4" y="4" width="92" height="92" fill="url(#${k}s)"/>
    </g>
    <rect x="4.5" y="4.5" width="91" height="91" rx="21.5" fill="none" stroke="#fff" stroke-opacity=".22"/>
  </svg>`;
}
