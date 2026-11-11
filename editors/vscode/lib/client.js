// Talking to the running Homebody: one JSON object per line over its private socket.
// No dependency on the editor, so it can be tested by itself.
'use strict';
const net = require('net');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const fs = require('fs');

/** Where Homebody keeps its things for this user, the same way the app works it out. */
function dataDir() {
  if (process.env.HOMEBODY_DATA_DIR) return process.env.HOMEBODY_DATA_DIR;
  const home = os.homedir();
  if (process.platform === 'darwin') return path.join(home, 'Library', 'Application Support', 'Homebody');
  if (process.platform === 'win32') return path.join(process.env.APPDATA || path.join(home, 'AppData', 'Roaming'), 'Homebody');
  return path.join(process.env.XDG_DATA_HOME || path.join(home, '.local', 'share'), 'Homebody');
}

/** The socket (macOS, Linux) or named pipe (Windows) the engine listens on. */
function socketPath() {
  if (process.platform === 'win32') {
    const tag = crypto.createHash('sha256').update(dataDir().toLowerCase()).digest('hex').slice(0, 12);
    return `\\\\.\\pipe\\homebody-control-${tag}`;
  }
  return path.join(dataDir(), 'control.sock');
}

/** One command, one answer. Rejects with a sentence when Homebody is not running or says no. */
function call(cmd, args, where = socketPath(), timeoutMs = 20000) {
  return new Promise((resolve, reject) => {
    const sock = net.connect(where);
    let buf = '', done = false;
    const finish = (fn, v) => { if (!done) { done = true; clearTimeout(timer); sock.destroy(); fn(v); } };
    const timer = setTimeout(() => finish(reject, new Error('Homebody did not answer in time.')), timeoutMs);
    sock.on('connect', () => sock.write(JSON.stringify({ cmd, args: args === undefined ? null : args }) + '\n'));
    sock.on('data', (d) => {
      buf += d.toString('utf8');
      const i = buf.indexOf('\n');
      if (i < 0) return;
      let v;
      try { v = JSON.parse(buf.slice(0, i)); } catch (e) { return finish(reject, new Error('Homebody answered with something unexpected.')); }
      if (v && v.ok) finish(resolve, v.result === undefined ? null : v.result);
      else finish(reject, new Error((v && v.error) || 'Homebody refused.'));
    });
    sock.on('error', () => finish(reject, new Error('Homebody is not running. Open the Homebody app.')));
    sock.on('close', () => finish(reject, new Error('Homebody closed the conversation.')));
  });
}

/**
 * Follow events ({type, data}) until `stop()` is called. Reconnects by itself when Homebody is
 * restarted; `onState(true|false)` says whether it is reachable.
 */
function subscribe(onEvent, onState, where = socketPath()) {
  let sock = null, stopped = false, retry = null, up = null;
  const state = (v) => { if (up !== v) { up = v; if (onState) onState(v); } };
  const connect = () => {
    if (stopped) return;
    let buf = '';
    sock = net.connect(where);
    sock.on('connect', () => { state(true); sock.write('{"cmd":"subscribe"}\n'); });
    sock.on('data', (d) => {
      buf += d.toString('utf8');
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i); buf = buf.slice(i + 1);
        try { const v = JSON.parse(line); if (v && v.event) onEvent(v.event); } catch (e) { /* skip a broken line */ }
      }
    });
    const again = () => { state(false); if (!stopped && !retry) retry = setTimeout(() => { retry = null; connect(); }, 3000); };
    sock.on('error', again);
    sock.on('close', again);
  };
  connect();
  return { stop() { stopped = true; clearTimeout(retry); if (sock) sock.destroy(); } };
}

/** The homebody command: the setting, inside the installed app, or just the name for the PATH. */
function findCli(configured) {
  if (configured && fs.existsSync(configured)) return configured;
  const inApp = '/Applications/Homebody.app/Contents/MacOS/homebody';
  if (process.platform === 'darwin' && fs.existsSync(inApp)) return inApp;
  return process.platform === 'win32' ? 'homebody.exe' : 'homebody';
}

/** Does `host` fall under `pattern` (a name, or *.name)? The same rule the engine uses. */
function matches(pattern, host) {
  pattern = String(pattern).toLowerCase(); host = String(host).toLowerCase();
  if (!pattern.startsWith('*.')) return pattern === host;
  const suffix = pattern.slice(2);
  return host.length > suffix.length + 1 && host.endsWith('.' + suffix);
}

/** Places a project file names: lower case, no duplicates, at most thirty. Never throws. */
function placesIn(text) {
  let v;
  try { v = JSON.parse(text); } catch (e) { return []; }
  const out = [];
  for (const h of (v && Array.isArray(v.trust) ? v.trust : [])) {
    if (typeof h !== 'string') continue;
    const name = h.trim().toLowerCase().replace(/\.$/, '');
    if (/^(\*\.)?([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]([a-z0-9-]{0,61}[a-z0-9])?$/.test(name) && !out.includes(name) && out.length < 30) out.push(name);
  }
  return out;
}

/** The few words for the status bar, from a status object. */
function summary(status) {
  if (!status) return { icon: 'warning', text: 'Homebody is not running', tip: 'Open the Homebody app to protect your AI agents.' };
  const pending = (status.pending || []).length;
  const ind = status.indicator;
  if (pending) return { icon: 'question', text: pending === 1 ? '1 question' : `${pending} questions`, tip: status.sentence };
  if (ind === 'attention' || status.setupNeeded) return { icon: 'warning', text: 'Needs attention', tip: status.sentence };
  if (status.lockdown) return { icon: 'circle-slash', text: 'Everything blocked', tip: status.sentence };
  if (ind === 'off') return { icon: 'unlock', text: 'Protection off', tip: status.sentence };
  if (ind === 'watching') return { icon: 'eye', text: 'Watching', tip: status.sentence };
  if (ind === 'blocked') return { icon: 'shield', text: 'Private · something was stopped', tip: status.sentence };
  return { icon: 'shield', text: 'Private', tip: status.sentence };
}

module.exports = { dataDir, socketPath, call, subscribe, findCli, matches, placesIn, summary };
