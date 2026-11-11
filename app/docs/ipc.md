# IPC contract: core ⇄ UI

The single source of truth for what the Rust core exposes and what the UI consumes. The UI never talks to anything else. In the Tauri app the transport is one command, `invoke('sai', { cmd, args })`, plus one event channel; in a plain browser (UI development) `ui/js/api.js` falls back to an in-page mock that implements this same contract.

All JSON is camelCase. Times are ISO 8601 UTC strings. Byte counts are integers.

## Vocabulary (user-facing words)

Everyday surfaces (Glance, Feed, Proof, prompts, onboarding) use only: **Private**, **Watching**, **Off**, **Trusted**, **Blocked**, **Allowed**, **Asked**, **Seen**, **Proof**. Never: egress, allowlist, policy, proxy, TLS, sandbox, firewall, pf. Those may appear only inside Studio → Advanced.

## Types

```ts
type Mode = "private" | "watch" | "off";

// The one-glance state for the menu bar dot and status sentence.
// private: all good. watching: Watch mode. asking: a prompt is waiting.
// blocked: something was blocked in the last 60 s. off: protection is off.
// attention: protection cannot be guaranteed (not set up, or the fence is down).
type Indicator = "private" | "watching" | "asking" | "blocked" | "off" | "attention";

type FenceState = "up" | "down" | "not_installed" | "checking";

interface Counters { bytesOut: number; bytesIn: number; allowed: number; blocked: number; asked: number; }

interface Status {
  version: string;
  mode: Mode;
  modeSince: string;            // when the current mode began
  privateSince: string | null;  // start of the current unbroken Private streak; null when not Private
  lockdown: boolean;            // panic button: everything blocked until released
  fence: FenceState;            // overall: "up" only if every protected app's fence is up
  setupNeeded: boolean;         // true when the one-time setup (admin approval) has not been done or must be repeated
  setupReasons: string[];       // plain sentences, e.g. "Protection was not restored after the last restart."
  indicator: Indicator;
  sentence: string;             // ready-made status line, e.g. "Private. 0 bytes left today."
  today: Counters;
  apps: AppStatus[];
  pending: Ask[];               // unanswered prompts, oldest first
  watch: WatchStatus | null;        // present while Watch is on
  watchReview: WatchStatus | null;  // a finished Watch has places waiting to be reviewed
}

interface AppStatus {
  id: string;                   // "ollama" | "claude-code"
  name: string;                 // "Ollama" | "Claude Code"
  kind: "model" | "agent";
  icon: string;                 // key into the UI's icon set: "ollama" | "claude" | "generic"
  installed: boolean;           // found on this computer
  protected: boolean;           // the user wants this app protected
  fence: FenceState;
  running: boolean;             // a protected instance started by us is running
  canStart: boolean;            // true when installed, protected and fence is up
  workspace: string | null;     // folder the app may write in (agents only)
  today: Counters;
  custom: boolean;              // added by the user ("protect another app"); can be removed
  problem: string | null;       // installed, but where a protected app cannot reach it (inside the home folder). A sentence for the user
}

type Verb = "allowed" | "blocked" | "asked" | "seen";

type Reason =
  | "trusted"            // destination is on the Trusted list
  | "not_trusted"        // not on the list, prompts disabled or suppressed
  | "you_allowed_once"   // user answered Allow once
  | "you_always_allow"   // user answered Always allow
  | "you_blocked"        // user answered Block
  | "no_answer"          // prompt timed out after 30 s → blocked
  | "watching"           // Watch mode: allowed and recorded
  | "protection_off"     // mode off: allowed and recorded
  | "lockdown"           // panic button active
  | "malformed"          // not a valid destination name (raw IP address, odd characters)
  | "local_address"      // name resolves to this computer or the local network
  | "around_gate";       // did not go through the gate; refused by the fence. host is "" and
                         // label says what was tried ("Direct connection", "Name lookup",
                         // "Opening another app", "Controlling another app", …)

interface FeedEvent {
  id: number;                   // stable id (ledger sequence number)
  ts: string;
  appId: string; appName: string; appIcon: string;
  host: string; port: number;
  label: string;                // plain name: "Anthropic API", or the host if unknown
  purpose: string;              // plain purpose: "Your AI provider", "Model download", "Usage statistics", "" if unknown
  verb: Verb;
  reason: Reason;
  explanation: string;          // one plain sentence the UI can show as-is
  open: boolean;                // connection still in progress
  bytesOut: number | null;      // null while open or when blocked
  bytesIn: number | null;
  durationMs: number | null;
  trusted: boolean;             // is this destination currently Trusted for this app
}

interface Ask {
  id: string;
  appId: string; appName: string; appIcon: string;
  host: string; port: number;
  label: string; purpose: string;
  title: string;                // "Ollama wants to reach ollama.com"
  body: string;                 // "Update check and model recommendations. Allow?"
  createdAt: string;
  expiresAt: string;            // createdAt + 30 s; unanswered → blocked
  waiting: number;              // how many connections are waiting on this answer
}

interface TrustEntry {
  appId: string;
  host: string;                 // exact name or "*.example.com"
  label: string; purpose: string;
  enabled: boolean;
  source: "preset" | "you";
  recommended: boolean;         // preset marks it as needed for the app to work
}

interface ChainCheck { ok: boolean; count: number; checkedAt: string; error: string | null; }

// Watch: on (Status.watch) or finished and waiting for a review (Status.watchReview).
interface WatchStatus {
  since: string;
  until: string | null;         // while on: when it switches itself back to Private (null: no timer).
                                // for a review: when it ended
  places: number;               // places contacted that are neither trusted nor blocked yet
}
interface WatchItem {
  appId: string; appName: string; appIcon: string;
  host: string; label: string; purpose: string;
  times: number; bytesOut: number;
}
interface WatchReview { since: string; until: string | null; active: boolean; items: WatchItem[]; }

// Projects: a folder brought in for an agent. The agent works on a copy inside its workspace;
// the original changes only on project_apply.
interface ProjectInfo {
  name: string;                 // folder name inside the workspace
  source: string;               // the user's original folder
  copy: string;                 // where the agent works
  importedAt: string;
  sourceExists: boolean; copyExists: boolean;
}
interface Change {
  path: string;                 // relative to the project
  kind: "added" | "modified" | "deleted";
  conflict: boolean;            // the user changed the same path since bringing the project in
  link: string | null;          // for a symbolic link: where it points
}
interface ChangeSet { total: number; conflicts: number; items: Change[]; truncated: boolean; }  // items capped at 500
interface ApplyResult { applied: number; skipped: string[]; }   // skipped: conflicting paths left alone

interface Proof {
  mode: Mode;
  privateSince: string | null;
  streakDays: number;           // whole days of the unbroken Private streak
  streakText: string;           // "14 days", "3 hours", "Just now"
  appsProtected: number;
  appNames: string[];
  bytesOutToday: number;        // bytes sent to Trusted destinations today
  untrustedBytesSincePrivate: number; // bytes that left to anything not Trusted since privateSince (0 is the goal)
  blockedTotal: number;         // blocked attempts since privateSince
  allowedTotal: number;
  chain: ChainCheck | null;     // last verification, null if never run
  publicKey: string;            // base64 Ed25519 public key of this computer's log
  statement: string;            // "Nothing left this Mac from Ollama and Claude Code to anywhere you have not trusted since 9 Nov 2026."
  covers: string;               // paragraph: what this proof covers
  cannotSee: string;            // paragraph: what it cannot see
}

interface SetupResult { ok: boolean; message: string; }
```

## Commands (`invoke(name, args)`)

| Command | Args | Returns | Notes |
|---|---|---|---|
| `get_status` | — | `Status` | |
| `set_mode` | `{ mode, minutes?: number }` | `Status` | switching away from Private ends the streak. `minutes` is for Watch: how long until it returns to Private by itself (default 60, 0 = until switched off); sending it while Watch is on moves the end time |
| `get_watch_review` | — | `WatchReview \| null` | what Watch has seen so far (active), or the finished Watch waiting for review |
| `finish_watch_review` | `{ trust: { appId, host }[] }` | `Status` | trusts the chosen places (only ones that were offered) and puts the review away; the rest stay undecided |
| `set_lockdown` | `{ on }` | `Status` | panic button |
| `get_feed` | `{ limit?: number, beforeId?: number, appId?: string, verb?: Verb }` | `FeedEvent[]` | newest first, default limit 200 |
| `decide` | `{ askId, choice: "once" \| "always" \| "block" }` | `null` | answers a prompt |
| `get_trust` | `{ appId?: string }` | `TrustEntry[]` | |
| `set_trust` | `{ appId, host, enabled }` | `TrustEntry[]` | creates the entry if missing (source "you") |
| `remove_trust` | `{ appId, host }` | `TrustEntry[]` | only entries with source "you" |
| `get_proof` | — | `Proof` | |
| `verify_chain` | — | `ChainCheck` | may take a moment on a large log |
| `export_proof` | — | `{ path: string }` | writes a signed proof file to Downloads and reveals it |
| `export_log` | `{ reveal?: boolean }` | `{ path: string }` | writes the whole log to Downloads: one signed header line, then every record as written. `sai verify <file>` checks it without the app |
| `get_receipts` | `{ limit?: number }` | `string[]` | raw log lines, newest last (Studio → Logs) |
| `get_policy_text` | — | `string` | raw settings as pretty JSON (Studio → Advanced, read-only) |
| `run_setup` | — | `SetupResult` | one-time setup; the OS asks for the admin password |
| `remove_setup` | — | `SetupResult` | undo everything setup created; asks for the admin password |
| `check_fence` | — | `Status` | re-run health probes now |
| `app_add` | `{ name, program }` | `Status` | protect another app. A command name or program path: it gets its own private user, gate and workspace, and setup must be approved again before it can run. A path ending in `.app`: a windowed app (`kind: "window"`), which runs as the user under the launch-time profile alone, needs no approval, and is opened and quit with `app_start` / `app_stop`. Rejects apps in Apple's App Sandbox with a sentence for the user |
| `app_remove` | `{ appId }` | `Status` | only apps added with `app_add`; also deletes its launcher |
| `discover_apps` | — | `FoundApp[]` | known windowed AI apps in the Applications folders that are not on the list yet. `FoundApp = { name, path, canProtect, asks, note }`: `asks` false means it can only run cut off from the internet; `note` is a sentence for the user (the reason, when `canProtect` is false) |
| `app_open` | `{ appId }` | `null` | open an app the way its kind is opened: a window or service is started, an agent gets a protected terminal. What a launcher calls |
| `app_launcher` | `{ appId, reveal?: boolean }` | `{ path }` | writes `~/Applications/Sandbox-ai/<Name> (protected).app`, a small app that opens this one protected (starting Sandbox-ai first if needed), and reveals it in Finder. Created automatically for windowed apps when they are added |
| `pick_program` | — | `string \| null` | shell only: the system file chooser |
| `get_login_item` / `set_login_item` | — / `{ on }` | `{ on: boolean }` | shell only: open the app at login (a per-user launch entry) |
| `app_set_protected` | `{ appId, protected }` | `Status` | |
| `app_start` | `{ appId }` | `Status` | model apps: start the protected instance |
| `app_stop` | `{ appId }` | `Status` | |
| `app_open_terminal` | `{ appId, project?: string }` | `null` | agents: open a protected terminal in the workspace, or in a project's copy |
| `get_projects` | `{ appId }` | `ProjectInfo[]` | |
| `project_import` | `{ appId, path }` | `ProjectInfo` | copies the folder into the workspace (instant, copy-on-write) and keeps a private baseline to compare against |
| `project_changes` | `{ appId, name }` | `ChangeSet` | what differs between the agent's copy and the baseline; `.git` is never compared |
| `project_apply` | `{ appId, name, overwrite?: boolean }` | `ApplyResult` | writes the changes into the original folder as the user. Conflicts are skipped unless `overwrite` |
| `project_remove` | `{ appId, name }` | `null` | deletes the copy and baseline; the original is not touched |
| `pick_folder` | — | `string \| null` | shell only: the system folder chooser; null when cancelled |
| `app_open_workspace` | `{ appId }` | `null` | reveal the workspace folder |
| `open_main` | `{ tab?: "feed" \| "proof" \| "studio" }` | `null` | show and focus the main window |
| `hide_window` | — | `null` | hides the calling window (Glance, prompt) |
| `quit` | — | `null` | |
| `get_setup_script` | — | `string` | exactly what the administrator approves in `run_setup`, for showing to the user first |
| `show` | `{ window: "main" \| "glance" \| "prompt", tab? }` | `null` | for the CLI and scripts: bring a window of the running app forward |

Errors reject with a plain string suitable for showing to the user.

## Events

One channel, `sai:event`. Payload is `{ type, data }`:

| `type` | `data` | When |
|---|---|---|
| `status` | `Status` | any state change (debounced to at most ~4 per second) |
| `feed` | `FeedEvent` | a new event, or an update to an existing one (same `id`: connection closed, verb changed after a prompt was answered) |
| `ask` | `Ask` | a new prompt, or `waiting` changed |
| `ask_resolved` | `{ id: string, choice: "once" \| "always" \| "block" \| "timeout" }` | prompt answered or expired |
| `navigate` | `{ tab: "feed" \| "proof" \| "studio" }` | main window should switch tab |

## Windows

| Window | File | Size | Chrome |
|---|---|---|---|
| Main | `ui/main.html` | 980×680, min 760×520 | native title bar overlay: traffic lights sit on our toolbar; the toolbar is a drag region (`data-tauri-drag-region`); leave 78 px on the left of the toolbar free for the traffic lights |
| Glance | `ui/glance.html` | 340×480 | frameless, transparent, native popover vibrancy behind it; the page draws nothing but its content on a transparent body; hides on blur |
| Prompt | `ui/prompt.html` | 380×128 | frameless, transparent, always on top, top-right of the screen; shows the oldest pending `Ask`; hides itself when none are pending |

## Rules the UI can rely on

- An `Ask` expires 30 s after `createdAt`; the core resolves it as blocked and emits `ask_resolved` with `timeout`. The UI shows the countdown but does not need to enforce it.
- "Allow once" permits that destination for that app for five minutes, so a burst of connections asks once.
- After Block or a timeout the core does not ask again for the same app and destination for ten minutes; those attempts appear in the Feed as blocked with reason `you_blocked` or `no_answer`.
- `set_trust` with `enabled: true` is "Always allow"; with `enabled: false` it is "Block". Both are instantly reversible by calling it again (Undo).
- While `status.setupNeeded` is true nothing protected can start, and the UI should lead with setup rather than the Feed.
