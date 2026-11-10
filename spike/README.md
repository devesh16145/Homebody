# Spikes

Throwaway-quality code that de-risks the product before the real repo exists. Each spike has its own README with status, how to run it, and what it proved.

- `fence/` — macOS Fence: keep a local model daemon serving localhost while denying its internet access, with signed receipts. Two implementations: pf (works today, needs sudo) and Network Extension (needs an Apple Developer identity). `fence/agent/` adds the no-VM agent fence: Claude Code as a dedicated macOS user behind pf and the allowlist proxy, driven from the web console.
- `box/` — macOS Box: Virtualization.framework VM with no network interface, vsock to a host allowlist proxy with signed receipts, virtiofs share, 3 s cold boot. Proven end to end from inside a Debian 13 guest; working image needs a reset from pristine after an editing mishap.

Specs these spikes serve: `../docs/product-spec.md`, `../docs/feature-spec.md`, `../docs/design-principles.md`.
