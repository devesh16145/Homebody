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

const TILES = { ollama: 'O', claude: 'C' };

/** Rounded monogram tile keyed by the contract's `icon` field. size: 16 | 20 | 28 | 36 */
export function appTile(key, name = '', size = 20) {
  const k = TILES[key] ? key : 'generic';
  const glyph = TILES[k] || (name ? esc(name.trim().charAt(0).toUpperCase()) : icon('spark'));
  return `<span class="tile t-${k}${size !== 20 ? ' s' + size : ''}" aria-hidden="true">${glyph}</span>`;
}

/** The product mark: a ring with a solid center. Used on setup and Proof. */
export function mark(size = 44) {
  return `<svg class="mark" width="${size}" height="${size}" viewBox="0 0 44 44" aria-hidden="true">
    <circle cx="22" cy="22" r="20" fill="var(--green-bg)"/>
    <circle cx="22" cy="22" r="12.5" fill="none" stroke="var(--green)" stroke-width="2.5"/>
    <circle cx="22" cy="22" r="5" fill="var(--green)"/></svg>`;
}
