# Design principles: fast, light, invisible, trusted

Status: v0.1, 2026-11-08. How the 88 features in `docs/feature-spec.md` become a product people love rather than a settings panel. Written from the stance of a consumer-product head: ChatGPT won with one text box, not with a feature list. This product wins with one switch, one feed and one proof.

## 1. The product in one sentence

Your AI stays on your computer, and you can prove it.

Everything in the interface serves that sentence. If a feature does not make the sentence more true or more believable, it does not get a visible surface.

## 2. The ten principles

1. One switch does the job. Installing and turning on Private mode must be the whole task for most people. No policy, no rules, no terms they have to learn.
2. Defaults do the work. We ship opinionated, curated defaults for every app we detect. Settings exist for the few who want them. Target: 90% of users never open settings.
3. Three layers, never one giant screen. Glance (menu bar), Feed (what happened), Studio (policy, logs, Box, CLI for developers). Each layer is complete on its own. Nobody is forced down a layer.
4. Silence is the feature. A security product that interrupts gets uninstalled. Hard budget of two prompts per user per day in Private mode; everything else is batched into the Feed. If we exceed it, our presets are wrong, not the user.
5. Speed is a feature with a budget. Menu bar opens in under 100 ms. App launch under 1 s. Decision latency under 2 ms. Box ready under 1 s warm. We measure these in CI and treat regressions as bugs.
6. Light enough to forget. Installer under 20 MB. Idle under 100 MB without a Box. No background CPU when nothing is happening. The Box gives memory back when idle.
7. Plain words. Private, Watching, Trusted, Blocked, Proof. Never egress, allowlist, policy, proxy, TLS or sandbox in the everyday surface. Technical words live in Studio.
8. Show, then ask. Every prompt tells the user what will happen in one line and offers at most three answers: Allow once, Always allow, Block. A fourth option is a design failure.
9. Proof is the hero, not the firewall. The emotional payoff is "14 days private, 0 bytes left." That streak is the home screen, the share card and the reason people tell a friend.
10. Honest by design. The product never says "unbreakable." The Proof screen shows what we cannot see, in the same plain words, so trust survives the first security blog post about us.

## 3. The first five minutes

The entire value must land before the user's attention runs out.

| Time | What happens | Why |
|---|---|---|
| 0:00 | Download, open. One screen: "Keep your AI on your computer." A single button: Turn on. | One decision. |
| 0:10 | macOS asks to allow the system extension. We explain in one sentence why, with a picture of the dialog. | The only unavoidable friction; make it feel expected. |
| 0:30 | We show the apps we found (Ollama, LM Studio, Jan, Claude Code) with checkmarks already on. "Protect these." | Zero configuration, visible value. |
| 0:45 | Private mode is on. Menu bar shows a green dot. Popover says "Private. Nothing has left since 0:45." | The product is working before the user has learned anything. |
| 2:00 | First real event: "LM Studio wanted to check for updates (lmstudio.ai). Blocked. Allow updates?" | The first prompt teaches the whole model in one line. |
| 5:00 | Feed shows a short list of what tried to leave and what happened. The Proof tab has started a streak. | The habit loop begins. |

Watch mode is the alternative entry for the cautious: everything allowed for an hour while we record, then "Here is what your AI contacted. Make it private?" with one button.

## 4. The three layers

Glance (menu bar popover, the default surface):
- Status dot and one sentence: "Private. 0 bytes left today."
- The Private mode switch.
- The last three events, each one line.
- Two buttons: Feed, Proof. A small gear for Studio.

Feed (one window, one list):
- Chronological events grouped by app, each one line with an icon and a verb: Blocked, Allowed, Asked.
- Tap an event for the plain-words explanation and the Allow or Block controls.
- Filters as chips, not forms: Today, Blocked, Per app.

Proof (the hero screen):
- The streak: "Private for 14 days." Bytes left: 0. Apps protected: 4.
- A Share button that creates a signed proof card (image plus verifiable file).
- "What this proof covers" and "What it cannot see" in two short paragraphs.

Studio (developers and the curious; everything else lives here):
- Trusted destinations as toggles with plain labels, with a raw policy editor behind an Advanced toggle.
- Box: run an agent in a sandbox, see its connections, snapshot and roll back.
- Logs: the full signed receipt view, verify and export.
- CLI and local API documentation.

All 88 features exist. About eight are visible in Glance and Feed. The rest live in Studio, where a developer finds them in seconds and an everyday user never needs to.

## 5. Interaction rules

- Every prompt fits in two lines and ends in a question. Example: "Jan wants to send 2 MB to huggingface.co to download a model. Allow?"
- Three answers maximum. Always allow is per app and per destination, never global.
- No modal dialogs over other apps. Prompts appear as a notification with inline buttons; if ignored for 30 seconds, the connection is blocked and logged, and the Feed shows it. Blocking by default on silence is the safe default and the honest one.
- Undo everywhere. Every Allow or Block is reversible from the Feed in one tap.
- Nothing nags. No upsell, no account, no rating prompt, no changelog pop-ups. Release notes live in Studio.
- Keyboard first in Studio. Every action has a shortcut; developers never reach for the mouse.
- Dark and light, native controls, system font, native menu bar. It should feel like a part of macOS, later Windows and Linux, not a web page in a frame.

## 6. What a head of product says no to (for v1)

- No chat interface. We are not another AI app; we protect them.
- No model store or model downloads of our own. We trust-label other apps' downloads instead.
- No account, no cloud sync, no telemetry we collect. The product's credibility depends on it.
- No general firewall positioning. We fence AI apps and agents by default; fencing everything is an opt-in in Studio.
- No browser extension at launch. Browser AI mode comes later via a fenced browser profile.
- No scoring, gamified badges or fear-based copy. The streak is the only game, and it is honest.
- No per-rule configuration in the everyday surface. If a user needs a rule, our preset has a bug.

## 7. Growth is built into the product, not bolted on

- The proof card is the share unit. Signed, verifiable, pretty, and it names the app ("Protected by Homebody").
- Presets are the contribution unit. Anyone can add an app in a small YAML file; the preset repo is where the community meets the product.
- Developers are the distribution unit. They install it for their agents, then tell their family it also keeps Jan private.
- The bypass corpus is the credibility unit. Every public test we publish is a reason a security engineer recommends us.

## 8. Metrics that matter

| Metric | Target | Why |
|---|---|---|
| Time to protected (download to green dot) | under 90 seconds median | Principle 1 |
| Prompts per user per day in Private mode | under 2 | Principle 4 |
| Users who ever open settings or Studio | under 10% of everyday users | Principle 2 |
| Menu bar open latency | under 100 ms p95 | Principle 5 |
| Idle memory without a Box | under 100 MB | Principle 6 |
| 30-day retention of installs | over 60% | A security tool that stays installed is the whole point |
| Proof cards shared per 100 users per month | 5 | The growth loop is working |
| Support questions containing a technical word from the banned list | trending to zero | Principle 7 |

## 9. The quality bar

Ship when a non-technical friend can install it, turn it on, understand the first prompt and explain to you what the product does, all without help, in under five minutes. Ship when a developer can wrap Claude Code in a Box in one command and finds nothing slower than before. Do not ship if either fails. Features wait; speed, lightness and clarity do not.
