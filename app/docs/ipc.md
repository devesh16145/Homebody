# IPC contract: core ⇄ UI

The single source of truth for what the Rust core exposes and what the UI consumes. The UI never talks to anything else. In the Tauri app the transport is one command, `invoke('homebody', { cmd, args })`, plus one event channel; in a plain browser (UI development) `ui/js/api.js` falls back to an in-page mock that implements this same contract.

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
  missed: Ask[];                // prompts nobody answered in time: blocked, and still answerable (`missed_answer`), oldest first
  askSound: boolean;            // a soft sound when a prompt appears
  ask: boolean;                 // false: places that are neither trusted nor blocked are refused without a prompt
  presetNotes: string[];        // what changed in the built-in rules since the user last said "Got it"; empty when nothing did
  localServers: boolean;            // agents may reach servers they started themselves (setup tested it here)
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
  inspect: boolean;             // looking inside is switched on
  canInspect: boolean;          // false for windowed apps and model servers: they take trusted certificates from the Mac
  keys: KeyInfo[];              // keys Homebody keeps for this app: { name, maker, hint, hosts, added }. Never a key
  secrets: boolean;             // requests carrying a secret are stopped (acts only while inspect is on)
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
  requests?: { what: string; times: number; path: string; label: string; allowed: boolean }[] | null;
                                // present when the connection was looked inside: "POST /v1/messages" and how often,
                                // most frequent first; `label` in plain words when known ("Conversation");
                                // `allowed` is the current switch for that path. Never anything after "?"
  secret?: { kind: string; fingerprint: string; allowed: boolean } | null;
  keys?: string[];              // names of kept keys that were put into requests on this connection
  keyStopped?: string;          // set when this row is a request stopped for carrying a kept key's stand-in somewhere the key is not for: the key's name
                                // present when this row is a request stopped for what its content carried. `kind` reads
                                // "a GitHub token" or "text from a folder you marked private"; the content itself is never stored,
                                // only this fingerprint
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
  pack?: string;                // the pack that brought this place, when one did (source "pack"): its id
  appId: string;
  host: string;                 // exact name or "*.example.com"
  label: string; purpose: string;
  enabled: boolean;
  source: "preset" | "you";
  recommended: boolean;         // preset marks it as needed for the app to work
  limitMb: number | null;       // megabytes this app may send here per hour; null: no limit
  sentMb: number | null;        // megabytes sent here in the last hour (one decimal), when a limit is set
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

// What each app did over the last days, counted from the log (Proof → "The last seven days").
interface Summary {
  days: number; since: string;
  apps: {
    appId: string; appName: string;
    allowed: number; blocked: number; bytesOut: number;
    aroundGate: number;          // attempts that did not go through the gate
    secretsStopped: number;      // requests stopped for carrying a secret or private text
    addressesRefused: number;    // requests to an address that is switched off
    places: { name: string; times: number; bytesOut: number }[];   // most used, at most five
    blockedPlaces: { name: string; times: number }[];              // most often blocked, at most five
  }[];
}

interface PrivateFolders {
  folders: { path: string; exists: boolean }[];
  files: number | null; lines: number | null;   // what the index holds; null until it has been built
}
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
| `set_limit` | `{ appId, host, mb?: number }` | `TrustEntry[]` | megabytes the app may send to this trusted place per hour; leave `mb` out to remove the limit. Counted by the gate for every app, with or without looking inside. Over the limit, new connections are refused and an open one is cut, until the hour has moved on |
| `set_ask` | `{ on }` | `Status` | off: unknown places are refused without a prompt, and prompts that are waiting end as if unanswered (blocked) |
| `set_ask_sound` | `{ on: boolean }` | `Status` | a soft sound when a question appears (on by default) |
| `missed_answer` | `{ id: string, choice: "always" \| "block" \| "later" }` | `Status` | answer a question that ran out of time (`Status.missed`): make the rule it would have made, or clear it so the app is asked about again next time |
| `ack_presets` | — | `Status` | the user has read `Status.presetNotes` |
| `get_private_folders` | — | `PrivateFolders` | |
| `add_private_folder` / `remove_private_folder` | `{ path }` | `PrivateFolders` | a folder whose text must never be sent. While an app's connections are looked inside, a request that repeats three or more long lines from a file in one of these folders is stopped. Must be a folder inside the user's home folder (not the home folder itself); at most 20 |
| `check_update` | — | `{ current, latest, newer, url, message }` | asks where Homebody is published whether a newer version exists. Only ever runs when the user asks; nothing is downloaded. `message` is a sentence for the user |
| `get_proof` | — | `Proof` | |
| `verify_chain` | — | `ChainCheck` | may take a moment on a large log |
| `export_proof` | — | `{ path: string }` | writes a signed proof file to Downloads and reveals it |
| `export_log` | `{ reveal?: boolean, dir?: string }` | `{ path: string }` | writes a folder to Downloads: `log.jsonl` (one signed header line, then every record as written), `activity.csv` (the Feed as a table), `rules.json` (the settings in force), `timestamps/` (any independent timestamps) and `README.txt`. `homebody verify <folder or log.jsonl>` checks it without the app |
| `timestamp_log` | — | `{ records, time, service, file }` | asks a public time-stamping service to vouch for the log as it stands. Only a fingerprint (a hash) of the log is sent, over the user's own connection, and the request is written to the log. The answer is kept beside the log and included in exports |
| `get_summary` | `{ days?: number }` | `Summary` | default 7, at most 90 |
| `get_receipts` | `{ limit?: number }` | `string[]` | raw log lines, newest last (Studio → Logs) |
| `get_policy_text` | — | `string` | raw settings as pretty JSON (Studio → Advanced, read-only) |
| `run_setup` | — | `SetupResult` | one-time setup; the OS asks for the admin password |
| `remove_setup` | — | `SetupResult` | undo everything setup created; asks for the admin password |
| `check_fence` | — | `Status` | re-run health probes now |
| `app_add` | `{ name, program }` | `Status` | protect another app. A command name or program path: it gets its own private user, gate and workspace, and setup must be approved again before it can run. A path ending in `.app`: a windowed app (`kind: "window"`), which runs as the user under the launch-time profile alone, needs no approval, and is opened and quit with `app_start` / `app_stop`. Rejects apps in Apple's App Sandbox with a sentence for the user |
| `app_remove` | `{ appId }` | `Status` | only apps added with `app_add`; also deletes its launcher |
| `discover_apps` | — | `FoundApp[]` | known windowed AI apps in the Applications folders that are not on the list yet. `FoundApp = { name, path, canProtect, asks, note }`: `asks` false means it can only run cut off from the internet; `note` is a sentence for the user (the reason, when `canProtect` is false) |
| `app_open` | `{ appId }` | `null` | open an app the way its kind is opened: a window or service is started, an agent gets a protected terminal. What a launcher calls |
| `app_launcher` | `{ appId, reveal?: boolean }` | `{ path }` | writes `~/Applications/Homebody/<Name> (protected).app`, a small app that opens this one protected (starting Homebody first if needed), and reveals it in Finder. Created automatically for windowed apps when they are added |
| `pick_program` | — | `string \| null` | shell only: the system file chooser |
| `get_install` | — | `{ path: string \| null, installed: boolean, canInstall: boolean }` | where the desktop app is running from, and whether that is `/Applications/Homebody.app` |
| `get_app_icons` | — | `{ [appId]: string }` | each protected app's own icon as a 128 px PNG `data:` URL, drawn by macOS from the app itself or, for a command-line agent, from its maker's Mac app (Claude for Claude Code, Cursor for the Cursor CLI…); apps without one are left out and shown as letter tiles. Cached in the data folder; nothing leaves the Mac |
| `install_app` | — | `{ path }` | copies the app into Applications, replacing an older copy |
| `install_and_restart` | — | `null` | shell only: `install_app`, then stop cleanly and reopen from Applications |
| `get_login_item` / `set_login_item` | — / `{ on }` | `{ on: boolean }` | shell only: open the app at login (a per-user launch entry) |
| `get_inside_rules` | `{ appId?: string }` | `InsideEntry[]` | switches for addresses inside trusted places. `InsideEntry = { appId, host, hostLabel, path, method, label, purpose, enabled, source }`; `path` is the start of a path, matched by whole segments; `method` is null for every kind of request, or one kind ("POST") |
| `set_inside_rule` | `{ appId, host, path, method?, enabled }` | `InsideEntry[]` | allow or refuse one address inside a place; creates the switch if missing (source "you"). Acts only while the app's connections are looked inside |
| `remove_inside_rule` | `{ appId, host, path, method? }` | `InsideEntry[]` | only switches with source "you" |
| `app_set_secrets` | `{ appId, on }` | `Status` | while looking inside, stop requests whose content holds something that looks like a private key or access token. On by default |
| `allow_secret` | `{ appId, fingerprint, allowed }` | `null` | let one particular secret through for this app from now on, or stop doing so. `fingerprint` comes from `FeedEvent.secret` |
| `get_keys` | `{ appId }` | `{ keys: KeyInfo[], known: { name, maker, hosts }[], canKeep, why }` | the keys Homebody keeps for an app (never the keys themselves), the key names it knows with their usual place, and whether a key can be kept for this app at all |
| `key_set` | `{ appId, name, value, hosts? }` | `Status` | keep a key: the app is handed a stand-in under `name`, and the real key is put into its requests at the gate, only for `hosts` (default: the usual place for a known name). Switches Look inside on. `value` is taken once and never returned |
| `key_remove` | `{ appId, name }` | `Status` | stop keeping it |
| `get_packs` | `{ appId }` | `{ id, name, what, hosts, downloadsOnly, on }[]` | the packs an agent can have switched on: the places a package manager fetches from, as one switch. Empty for anything that is not an agent |
| `set_pack` | `{ appId, pack, on }` | `TrustEntry[]` | switch a pack on or off. On: each of its places becomes a rule with source `pack` (and, for a downloads-only pack, an hourly limit of 2 MB on what may be sent there). Off: only what the pack brought is taken away |
| `leak_test` | `{ appId }` | `{ appId, appName, at, held, through, unknown, cases: { id, what, outcome }[] }` | try to get out as the app (straight to the internet, a name lookup, a bare address through the gate, listing your home folder and your SSH keys, reading a file of yours by name, writing to your home) and say what became of each attempt. `outcome` is `"stopped"` only when the refusal itself was seen, `"through"` when the attempt succeeded, and `"unknown"` otherwise (it did not finish, or was ended from outside); `held` is true only when every attempt was seen to be stopped. The attempts that go around the gate are not written down as the app's; the one through the gate is, as any refused address is |
| `app_set_inspect` | `{ appId, on }` | `Status` | look inside this app's connections to trusted places (see "Looking inside" in the README). Switching on creates this computer's certificate authority if needed and puts its certificate where protected apps can read it. Rejects apps that cannot support it with a sentence for the user |
| `app_set_protected` | `{ appId, protected }` | `Status` | |
| `app_start` | `{ appId }` | `Status` | model apps: start the protected instance |
| `app_stop` | `{ appId }` | `Status` | |
| `app_open_terminal` | `{ appId, project?: string }` | `null` | agents: open a protected terminal in the workspace, or in a project's copy |
| `get_projects` | `{ appId }` | `ProjectInfo[]` | |
| `project_import` | `{ appId, path }` | `ProjectInfo & { suggested }` | copies the folder into the workspace (instant, copy-on-write) and keeps a private baseline to compare against. `suggested: { host, label, purpose }[]` lists the places the folder's `.homebody.json` asks for that are not trusted yet; nothing is trusted until the user agrees (`set_trust`) |
| `project_changes` | `{ appId, name }` | `ChangeSet` | what differs between the agent's copy and the baseline; `.git` is never compared |
| `project_apply` | `{ appId, name, overwrite?: boolean }` | `ApplyResult` | writes the changes into the original folder as the user. Conflicts are skipped unless `overwrite` |
| `project_remove` | `{ appId, name }` | `null` | deletes the copy and baseline; the original is not touched |
| `project_restart` | `{ appId, name }` | `ProjectInfo` | throw the copy away, with whatever the agent did to it, and bring the original in again as it is now. The original is not touched |
| `pick_folder` | — | `string \| null` | shell only: the system folder chooser; null when cancelled |
| `app_open_workspace` | `{ appId }` | `null` | reveal the workspace folder |
| `open_main` | `{ tab?: "feed" \| "proof" \| "studio" }` | `null` | show and focus the main window |
| `hide_window` | — | `null` | hides the calling window (Glance, prompt) |
| `quit` | — | `null` | |
| `fenced_argv` | `{ appId, program, args?: string[], cwd?: string, env?: { [name]: string } }` | `string[]` | the full command line that runs a program inside this app's fence; what `homebody run` uses |
| `get_setup_script` | — | `string` | exactly what the administrator approves in `run_setup`, for showing to the user first |
| `show` | `{ window: "main" \| "glance" \| "prompt", tab? }` | `null` | for the CLI and scripts: bring a window of the running app forward |
| `get_ai_sites` | `{ appId?: string }` | `{ browsers: { name, path, appId }[], sites: { id, name, url, on, places }[] }` | browsers that can have a profile kept for AI websites, and the sites; `on` is for `appId` |
| `browser_add` | `{ browser, sites: string[] }` | `Status & { addedId }` | a separate profile of that browser, fenced, reaching only those sites |
| `browser_set_site` | `{ appId, site, on }` | `TrustEntry[]` | switch one AI site on or off for a browser profile |
| `export_policy` | `{ dir?: string, reveal?: boolean }` | `{ path }` | the rules as Cedar policies with a schema and entities; also part of `export_log` |
| `fleet_status` | — | `{ joined, org?, url?, lastSync?, lastError?, sent?, waiting?, policyVersion?, rules?, locked? }` | whether this computer is in an organisation's fleet |
| `fleet_join` | `{ url, code, caFile?: string }` | as `fleet_status` | join: its rules then come first and the log is sent to it |
| `fleet_sync` / `fleet_leave` | — | as `fleet_status` | talk to the server now / leave it (its rules stop applying) |
| `box_status` | `{ appId?: string }` | `{ supported, helper, prepared, running, appId?, appName?, share?, since?, busy?: { what: "setup"\|"install", appId?, appName?, step, since, downloadedMb?, downloadMb }, last?: { what, appId?, ok, message, at }, installed: { [appId]: version }, downloadMb, agent?: { id, canInstall, installed? } }` | the Box (Apple silicon). `busy` is a long step in progress; `last` is how the last one ended; `agent` is there when an app is named |
| `box_setup` | `{ appId?: string, image?: string, background?: boolean }` | `{ message, status }`, or `box_status` at once with `background` | make the Box's disk; long the first time |
| `box_install` | `{ appId, background?: boolean }` | `{ message }`, or `box_status` at once with `background` | put an agent into the prepared disk, through its gate |
| `box_open` | `{ appId, project?: string }` | as `box_status` | start the Box for this agent if it is not running, and open a Terminal window in it with the agent |
| `box_dismiss` | `{ appId?: string }` | as `box_status` | forget how the last long step ended |
| `box_start` | `{ appId, project?: string }` | as `box_status` | start it for one agent, sharing that agent's folder or one project |
| `box_stop` | — | as `box_status` | stop it and throw its disk away |
| `box_argv` | `{ program, args?: string[], interactive?: boolean }` | `string[]` | the command line that runs something inside the running Box |

Errors reject with a plain string suitable for showing to the user.

## Events

One channel, `homebody:event`. Payload is `{ type, data }`:

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
| Main | `ui/home.html` | 980×680, min 760×520 | native title bar overlay: the traffic lights sit on the floating sidebar, whose top is a drag region (`data-tauri-drag-region`), as is a strip across the top of the page |
| Menu-bar panel | `ui/menubar.html` | 340×480 | window label `glance`; frameless, transparent, native popover vibrancy behind it; the page draws nothing but its content on a transparent body; hides on blur |
| Prompt | `ui/prompt.html` | 380×128 | frameless, transparent, always on top and on every desktop, top-right of the screen the pointer is on; shows the oldest pending `Ask`, then any `missed` ones, which stay until dealt with; hides itself when there are none |

## Rules the UI can rely on

- An `Ask` expires 30 s after `createdAt`; the core resolves it as blocked and emits `ask_resolved` with `timeout`. The UI shows the countdown but does not need to enforce it.
- "Allow once" permits that destination for that app for five minutes, so a burst of connections asks once.
- After Block or a timeout the core does not ask again for the same app and destination for ten minutes; those attempts appear in the Feed as blocked with reason `you_blocked` or `no_answer`.
- `set_trust` with `enabled: true` is "Always allow"; with `enabled: false` it is "Block". Both are instantly reversible by calling it again (Undo).
- While `status.setupNeeded` is true nothing protected can start, and the UI should lead with setup rather than the Feed.
