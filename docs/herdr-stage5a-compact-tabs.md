# 5A — Compact Herdr-style tabs

This update changes **only the tab strip presentation** in Gentle Shell Desktop,
using the supplied Herdr screenshot as the visual reference.

## Appearance

- Compact, numbered tabs (`1`, `2`, `3` …) in the existing title bar.
- Outlined active tab with a subtle pink accent; muted inactive tabs.
- A small type icon (agent, terminal, file) or the existing working indicator.
- An always-visible `×` on every tab, and the existing `+` menu.
- Hover text preserves the full conversation title, machine name and split-pane
  count, which no longer occupy horizontal tab-strip space.
- Double-click continues to open inline agent-session renaming; the field
  temporarily expands the tab without changing the underlying session.
- IDs remain stable across tab closures; only the visual ordinal changes.

## Functionality deliberately unchanged

- Ctrl+T / Ctrl+Shift+T and all existing workspace actions.
- Existing `+` menu for agent, remote-machine agent and terminal.
- `close_tab`, `activate`, split-pane focus and reorder handling.
- Per-machine Herdr selection, local/native fallback, agent addressing,
  remote reconnection and termination; these systems are not modified.
- Tab/workspace persistence; no schema migration.

## Windows verification

```powershell
git fetch origin
git switch feature/herdr-stage5a-compact-tabs
git pull --ff-only origin feature/herdr-stage5a-compact-tabs
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p gentle-shell-gpui
```

1. Create three tabs (local agent, JaviPi agent, terminal). They should be
   numbered 1–3; type icons and tab-specific title/machine tooltips should
   be correct.
2. Activate each tab and confirm the selected tab gets a pink border.
3. Close the second tab via its visible ×; only that agent/terminal must
   terminate, and the remaining tabs should show 1 and 2.
4. Test Ctrl+T, Ctrl+Shift+T, keyboard tab switching and the + dropdown.
5. Double-click an agent tab, rename it, and verify the updated name appears
   in the tooltip and session without changing the compact tab width.
6. Create a split-pane tab and verify both panes still work and the tooltip
   shows its pane count.
7. Restart GPUI, confirm restored tabs and remote `javipi/tN` identities,
   and verify there are no duplicate agents.
8. Repeat in native `local` mode to ensure the UI change does not depend
   on Herdr.

**Validation status:** The change was prepared in a feature branch. Rust
Clippy, Rust tests and Windows visual acceptance must run before merge.
