# Stage 8C — Local / Herdr action parity

This audit is based on the actual desktop implementation and current Herdr bridge command set, not just identical-looking panels. The goal is one user-facing workflow with backend-specific I/O and **no Windows Git or filesystem fallback for a JaviPi agent**.

## Code audit

| Area | Shared local / Herdr behavior | 8C changes | Remaining differences or verification |
| --- | --- | --- | --- |
| Files | Same Files/Changes/Branches dock, editing, tree navigation, create/rename/copy/move/trash, transfer actions; remote POSIX paths and session-bound RPC | Remote changed-file **Open file** now uses the existing Linux editor path, never local Git | Check rename, clipboard and transfer round-trips on Windows/JaviPi; remote editors remain revision-checked; capability limits and file sizes differ |
| Changes | Shared list/tree, diffs, stage/unstage, bulk stage, stash, commit | Remote right-click now has **Open file, Stage/Unstage, Discard** like local. One native discard dialog, then exact guarded Herdr write; remote commit now accepts **summary + description**; empty stash message uses a valid WIP default | Local **Commit & Push**, **Amend**, and **Write it** are **not yet equivalent** in the shared remote commit UI; do not silently call local Git |
| Branches | Shared branch/tag/stash/worktree list; remote guarded branch switch, stash, worktree and network actions | Remote footer now shows upstream/ahead-behind metadata and **Git History** button via active agent; never turn Linux paths into Windows paths | Local worktree creation opens a local agent automatically; remote worktree action may need explicit remote-agent launch. Exact network/confirmation parity requires dedicated testing |
| History | Shared History graph, filters and pagination with separate local Git versus remote `herdr_git_history` | Branches -> History now works for the active Herdr agent | Older remote bridge may not serve `herdr_git_history`; it must report unavailable rather than use local Git |
| Profiles | Same Profiles UI; session-bound Desktop configuration RPC for eligible Shells | Audited; no change necessary in this batch | Old remote Shell without configuration RPC cannot be configured from Windows files. This is intentionally blocked, not silently emulated |
| Palette | Same categories and navigation; remote queries are scoped to machine, root and terminal, avoiding stale selections | Audited; no change necessary in this batch | Some commands are inherently about this **Windows desktop**, not the remote Shell; do not treat Settings/Providers/Setup as remote configuration |
| Agents and sessions | Session-scoped Pi RPC, Herdr reconnect and persistent agent/terminal identity | Audited; no change necessary in this batch | Confirm reconnection, multiple machines, local/remote focused panes, and remote bridge capability negotiation in live QA |

**Parity means identical supported user actions, not identical underlying commands.** The bridge must keep its existing source-specific revision checks and prevent cross-machine actions.

## 8C implementation notes

- `ChangesEvent::RemoteOpenFile` is translated into a validated repository-relative POSIX path before `RemoteFilesEvent::OpenFile`; the same helper hardens remote diff navigation.
- A local Changes context menu continues to use native `git` and `PathBuf`; a remote menu emits session-bound Herdr RPC events.
- The shared destructive-action confirmation is the single user confirmation for both backends. Remote `herdr_git_discard` still verifies HEAD, branch, index revision, exact selected path and porcelain status inside the bridge. No unguarded `git checkout --` runs on Windows.
- Remote commit's reviewed pair is now **(summary, description)**: editing either between the two confirmation clicks requires confirming the new message. The bridge accepts a description of at most 4,000 bytes and checks HEAD/index.
- The remote stash dialog labels its message optional; an empty or whitespace-only message becomes `WIP (Gentle Shell)` to satisfy the bridge's non-empty `-m` requirement.
- Branches -> History emits a typed remote event and is checked against the active session in Workspace before opening the same remote History pane.

## Acceptance checks (Windows PowerShell)

```powershell
git fetch origin
git switch feature/herdr-stage8c-action-parity
git pull --ff-only origin feature/herdr-stage8c-action-parity

cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings

$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
cargo run --locked -- .
```

Use a **disposable test repository on JaviPi**, not a project with uncommitted work you need to retain.

1. Open one local Windows agent and one JaviPi agent, each pointed at a different Git repository. Switch back and forth with the compact Machines rail. The Files/Changes/Branches panels must follow the active agent.
2. On JaviPi, right-click a modified file in **Changes**. Verify `Open file` opens its **Linux** file in a central pane. Verify `Stage`, `Unstage`, and the checkbox act on JaviPi only.
3. Use **Discard changes…** on a test file. Cancel must do nothing. Confirm once and verify the reviewed changes are discarded only in the remote repository. If another process modifies Git index/HEAD after the snapshot, the bridge should reject the stale operation.
4. Stage a changed file, enter a subject **and a multiline description**, press Commit then Confirm. Inspect `git log -1 --format=%B` on JaviPi and check both parts are preserved. Change either field between clicks to ensure the confirmation resets.
5. Modify a disposable file and **Stash all changes** with an empty message: it should create a stash with `WIP (Gentle Shell)`. Non-empty messages should remain unchanged.
6. From **Branches** on JaviPi, click the Git History graph button in the footer. It must open a remote History pane tied to the selected agent (no Windows Git history).
7. Open a branch and History on Windows and verify the preexisting native behavior remains unchanged. Check the remote Profiles and Palette screens read their own source, without borrowing Windows settings.
8. Disconnect JaviPi during a read or mutation, reconnect through Herdr, and confirm remote editors, sessions and latest Git status remain correctly scoped.

Do **not** merge 8C into `main` until compiler, clippy, and these real local/remote tests are green. The outstanding differences above remain part of the broader parity roadmap, rather than being labeled done.
