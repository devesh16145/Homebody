// Name directions under review. Each has a mark (with states for the menu bar) and an app icon.
// The chosen one moves into icons.js; the others are dropped.

/* ---------- Homebody: a small round face peeking over the windowsill. It stays in. ---------- */

function homebodyMark(size = 44, state = 'private', id = 'hb') {
  const k = `${id}${size}${state}`;
  const headY = state === 'shut' ? 50 : 40;           // sealed: ducked down, only the top shows
  const look = state === 'ask' ? [2.2, -2.2] : [0, 0];   // asking: looks up at you
  const fill = state === 'ask' ? 'var(--ask)' : state === 'off' ? 'none' : 'currentColor';
  const eyes = state === 'shut' ? '' : `<ellipse cx="${25 + look[0]}" cy="${31 + look[1]}" rx="2.7" ry="3.8"/><ellipse cx="${39 + look[0]}" cy="${31 + look[1]}" rx="2.7" ry="3.8"/>`;
  return `<svg class="brand-mark" width="${size}" height="${size}" viewBox="0 0 64 64" aria-hidden="true" style="color:${state === 'off' ? 'var(--ink-3)' : 'var(--tide)'}">
    <defs><mask id="${k}m"><rect width="64" height="64" fill="#fff"/><g fill="#000">${eyes}</g></mask>
      <clipPath id="${k}c"><rect width="64" height="40"/></clipPath></defs>
    <g clip-path="url(#${k}c)"><path d="M12 ${headY}A20 20 0 0 1 52 ${headY}Z" fill="${fill}" ${state === 'off' ? 'stroke="currentColor" stroke-width="3.5"' : ''} mask="url(#${k}m)"/>
      ${state === 'off' ? `<g fill="currentColor">${eyes}</g>` : ''}</g>
    <rect x="6" y="40" width="52" height="6" rx="3" fill="currentColor"/>
    ${state === 'shut' ? '' : '<circle cx="17.5" cy="40.5" r="3.6" fill="currentColor"/><circle cx="46.5" cy="40.5" r="3.6" fill="currentColor"/>'}
  </svg>`;
}

function homebodyIcon(size = 128, id = 'hbi') {
  const k = id + size;
  return `<svg width="${size}" height="${size}" viewBox="0 0 100 100" aria-hidden="true">
    <defs>
      <linearGradient id="${k}g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5ad6de"/><stop offset="1" stop-color="#077a86"/></linearGradient>
      <linearGradient id="${k}f" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#d9f3f5"/></linearGradient>
      <linearGradient id="${k}s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".28"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient>
      <clipPath id="${k}t"><rect x="4" y="4" width="92" height="92" rx="22"/></clipPath>
    </defs>
    <g clip-path="url(#${k}t)">
      <rect x="4" y="4" width="92" height="96" fill="url(#${k}g)"/>
      <path d="M22 66A28 28 0 0 1 78 66Z" fill="url(#${k}f)"/>
      <ellipse cx="40" cy="52" rx="3.8" ry="5.3" fill="#0b1620"/><ellipse cx="60" cy="52" rx="3.8" ry="5.3" fill="#0b1620"/>
      <rect x="4" y="66" width="92" height="34" fill="#0b1620" fill-opacity=".86"/>
      <rect x="4" y="66" width="92" height="2" fill="#fff" fill-opacity=".25"/>
      <circle cx="29" cy="66.5" r="5.2" fill="url(#${k}f)"/><circle cx="71" cy="66.5" r="5.2" fill="url(#${k}f)"/>
      <rect x="4" y="4" width="92" height="92" fill="url(#${k}s)"/>
    </g>
    <rect x="4.5" y="4.5" width="91" height="91" rx="21.5" fill="none" stroke="#fff" stroke-opacity=".22"/>
  </svg>`;
}

/* ---------- Ebb: a glass vial of still water. The level is yours to set. ---------- */

const WAVE = (y, x0 = 4, step = 7) => `M${x0} ${y}q${step / 2}-4.5 ${step} 0t${step} 0t${step} 0t${step} 0t${step} 0t${step} 0t${step} 0t${step} 0`;
const VIAL = 'M20 18A12 12 0 0 1 44 18V46A12 12 0 0 1 20 46Z';

function ebbMark(size = 44, state = 'private', id = 'eb') {
  const k = `${id}${size}${state}`;
  const level = state === 'shut' ? 16 : 34;
  const water = state === 'ask' ? 'var(--ask)' : 'currentColor';
  return `<svg class="brand-mark" width="${size}" height="${size}" viewBox="0 0 64 64" aria-hidden="true" style="color:${state === 'off' ? 'var(--ink-3)' : 'var(--tide)'}">
    <defs><clipPath id="${k}c"><path d="${VIAL}"/></clipPath></defs>
    ${state === 'off' ? '' : `<path clip-path="url(#${k}c)" d="${WAVE(level)}V64H4Z" fill="${water}"/>`}
    <path d="${VIAL}" fill="none" stroke="currentColor" stroke-width="5"/>
  </svg>`;
}

function ebbIcon(size = 128, id = 'ebi') {
  const k = id + size;
  const vial = 'M36 26A14 14 0 0 1 64 26V72A14 14 0 0 1 36 72Z';
  return `<svg width="${size}" height="${size}" viewBox="0 0 100 100" aria-hidden="true">
    <defs>
      <linearGradient id="${k}g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#173444"/><stop offset="1" stop-color="#07121a"/></linearGradient>
      <linearGradient id="${k}w" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#7ff0e3"/><stop offset="1" stop-color="#0a8d99"/></linearGradient>
      <linearGradient id="${k}s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".2"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient>
      <clipPath id="${k}c"><path d="${vial}"/></clipPath>
    </defs>
    <rect x="4" y="4" width="92" height="92" rx="22" fill="url(#${k}g)"/>
    <path d="${vial}" fill="#fff" fill-opacity=".07"/>
    <path clip-path="url(#${k}c)" d="${WAVE(52, 20, 9)}V100H20Z" fill="url(#${k}w)"/>
    <path d="${vial}" fill="none" stroke="#eaf8fa" stroke-width="5"/>
    <path d="M43 28V44" stroke="#fff" stroke-opacity=".55" stroke-width="3" stroke-linecap="round"/>
    <rect x="4" y="4" width="92" height="92" rx="22" fill="url(#${k}s)"/>
    <rect x="4.5" y="4.5" width="91" height="91" rx="21.5" fill="none" stroke="#fff" stroke-opacity=".16"/>
  </svg>`;
}

/* ---------- Inlet: a lagoon inside a reef, with one narrow channel. ---------- */

function ring(r, gapDeg, centreDeg) {
  const c = 2 * Math.PI * r, gap = c * gapDeg / 360;
  const start = centreDeg + gapDeg / 2;
  return `stroke-dasharray="${(c - gap).toFixed(2)} ${gap.toFixed(2)}" transform="rotate(${start} 32 32)"`;
}

function inletMark(size = 44, state = 'private') {
  const gap = state === 'shut' ? 0.001 : state === 'off' ? 150 : 46;
  const core = state === 'ask' ? 'var(--ask)' : state === 'off' ? 'none' : 'currentColor';
  return `<svg class="brand-mark" width="${size}" height="${size}" viewBox="0 0 64 64" aria-hidden="true" style="color:${state === 'off' ? 'var(--ink-3)' : 'var(--tide)'}">
    <circle cx="32" cy="32" r="22" fill="none" stroke="currentColor" stroke-width="8" stroke-linecap="round" ${ring(22, gap, -45)}/>
    <circle cx="32" cy="32" r="10.5" fill="${core}" ${state === 'off' ? 'stroke="currentColor" stroke-width="3"' : ''}/>
  </svg>`;
}

function inletIcon(size = 128, id = 'ini') {
  const k = id + size;
  return `<svg width="${size}" height="${size}" viewBox="0 0 100 100" aria-hidden="true">
    <defs>
      <linearGradient id="${k}g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#dbe7ec"/></linearGradient>
      <radialGradient id="${k}c" cx=".38" cy=".34" r=".75"><stop offset="0" stop-color="#b9fff6"/><stop offset=".5" stop-color="#2cc3cf"/><stop offset="1" stop-color="#06707b"/></radialGradient>
      <linearGradient id="${k}r" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#14a3b0"/><stop offset="1" stop-color="#065f69"/></linearGradient>
    </defs>
    <rect x="4" y="4" width="92" height="92" rx="22" fill="url(#${k}g)"/>
    <g transform="translate(13.6 13.6) scale(1.14)">
      <circle cx="32" cy="32" r="22" fill="none" stroke="url(#${k}r)" stroke-width="8" stroke-linecap="round" ${ring(22, 46, -45)}/>
      <circle cx="32" cy="32" r="10.5" fill="url(#${k}c)"/>
    </g>
    <rect x="4.5" y="4.5" width="91" height="91" rx="21.5" fill="none" stroke="#0e1a22" stroke-opacity=".08"/>
  </svg>`;
}

export const BRANDS = [
  {
    id: 'homebody', name: 'Homebody', font: "'Fredoka', var(--font-round)", weight: 600, tracking: '-.01em', lower: false,
    line: 'Your AI stays home.',
    idea: 'A small face peeking over the windowsill. It watches the world but stays in.',
    voice: ['Claude Code stayed home today.', 'Ollama wants to go out to ollama.com.', 'Everyone’s inside. Nothing left.'],
    mark: homebodyMark, icon: homebodyIcon,
    states: [['private', 'Home'], ['ask', 'Asking'], ['shut', 'Ducked down'], ['off', 'Out']],
  },
  {
    id: 'ebb', name: 'ebb', font: "'Instrument Serif', Georgia, serif", weight: 400, tracking: '-.01em', lower: true,
    line: 'Nothing flows out unless you let it.',
    idea: 'A glass vial of still water. Your AI lives in it; you decide what goes out with the tide.',
    voice: ['Calm water. 5 stopped today.', 'Ollama wants to send to ollama.com.', 'Tide held. Nothing leaves.'],
    mark: ebbMark, icon: ebbIcon,
    states: [['private', 'Calm'], ['ask', 'Asking'], ['shut', 'Held'], ['off', 'Open sea']],
  },
  {
    id: 'inlet', name: 'Inlet', font: "'Unbounded', var(--font)", weight: 600, tracking: '-.03em', lower: false,
    line: 'One way out, and it’s yours.',
    idea: 'A lagoon inside a reef. One narrow channel is the only way out, and you control it.',
    voice: ['Channel open for 3 places.', 'Ollama wants through to ollama.com.', 'Channel closed. Nothing leaves.'],
    mark: inletMark, icon: inletIcon,
    states: [['private', 'Open to 3'], ['ask', 'Asking'], ['shut', 'Closed'], ['off', 'No reef']],
  },
];
