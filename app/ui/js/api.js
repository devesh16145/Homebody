// The only module that touches the transport.
// In the app: Tauri invoke + the single `homebody:event` channel.
// In a plain browser: the in-page mock (shared with sibling iframes under dev.html).

const tauri = typeof window !== 'undefined' ? window.__TAURI__ : null;
const windowName = document.documentElement.dataset.window || 'main';

let mockPromise = null;
function getMock() {
  if (!mockPromise) {
    let shared = null;
    try { if (window.parent !== window) shared = window.parent.__HB_MOCK__ || null; } catch (e) { /* cross-origin parent */ }
    mockPromise = shared ? Promise.resolve(shared) : import('./mock.js').then((m) => {
      const scenario = new URLSearchParams(location.search).get('scenario') || 'private';
      window.__HB_MOCK__ = m.createMock(scenario);
      return window.__HB_MOCK__;
    });
  }
  return mockPromise;
}

// Objects from the shared mock live in the parent frame's realm; copy them into ours.
const local = (v) => (v == null ? null : JSON.parse(JSON.stringify(v)));

/** Call a core command. Resolves with the result, rejects with a plain string. */
export async function invoke(cmd, args) {
  // The shell exposes one command; the core dispatches on `cmd` exactly as in docs/ipc.md.
  if (tauri) return tauri.core.invoke('homebody', { cmd, args: args ?? null });
  const mock = await getMock();
  return local(await mock.invoke(cmd, args, { window: windowName }));
}

/** Subscribe to core events. The handler receives `{ type, data }`. Returns an unsubscribe function. */
export function onEvent(handler) {
  let off = null, cancelled = false;
  const stop = () => { cancelled = true; if (off) off(); };
  if (tauri) {
    tauri.event.listen('homebody:event', (e) => handler(e.payload)).then((un) => { off = un; if (cancelled) un(); });
  } else {
    getMock().then((mock) => {
      if (cancelled) return;
      off = mock.subscribe((evt) => handler(local(evt)));
    });
    window.addEventListener('pagehide', stop, { once: true });
  }
  return stop;
}
