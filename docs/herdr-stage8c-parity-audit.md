# Stage 8C — Local / Herdr action parity

This audit is based on the actual desktop implementation and current Herdr bridge command set, not just identical-looking panels. The goal is one user-facing workflow with backend-specific I/O and **no Windows Git or filesystem fallback for a JaviPi agent**.

## Code audit

| Area | Shared local / Herdr behavior | 8C changes | Remaining differences or verification |
| --- | --- | --- | --- |
| Files | Same Files/Changes/Branches dock, editing, tree navigation, create/rename/copy/move/trash, transfer actions; remote POSIX paths and session-bound RPC | Remote changed-file **Open file** now uses the existing Linux editor path, never local Git | Check rename, clipboard and transfer round-trips on Windows/JaviPi; remote editors remain revision-checked; capability limits and file sizes differ |
| Changes | Shared list/tree, diffs, stage/unstage, bulk stage, stash, commit | Remote right-click now has **Open file, Stage/Unstage, Discard** like local. One native discard dialog, then exact guarded Herdr write; remote **summary + description**, **Amend**, **Commit & Push** and **Write it** use Herdr only; empty stash message uses a valid WIP default | Test remote CLI message generation on ARM64 (extra headless process), network credentials and failure after commit; older Herdr agents require a fresh bridge |
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
- Remote commit confirmation is bound to **(summary, description, amend, push)**: changing the message, Amend mode or push action invalidates the previous confirmation. The bridge accepts a description of at most 4,000 bytes and checks HEAD/index. Successful commits clear the message and Amend mode; a commit followed by failed push refreshes the changed HEAD and shows the partial success.
- **Commit & Push** preflights a safe configured remote before writing. The bridge performs a non-forced push of the selected Linux branch after commit; the initial commit path resolves its new HEAD. No Windows Git runs.
- **Amend** is explicit, requires existing HEAD, can rewrite a message with no staged changes, and prefills the old message if fields were blank. Switching branches resets Amend mode.
- **Write it** uses a bounded staged Linux diff, then an explicit headless remote Gentle Shell invocation with `--print --no-session --no-tools`. The returned message is rejected if the Git index or HEAD changed while it was generated. No Pi bash tool call or local model fallback occurs.
- The remote stash dialog labels its message optional; an empty or whitespace-only message becomes `WIP (Gentle Shell)` to satisfy the bridge's non-empty `-m` requirement.
- Branches -> History emits a typed remote event and is checked against the active session in Workspace before opening the same remote History pane.

## Acceptance checks (Windows PowerShell)

```powershell
git fetch origin
git switch feature/herdr-stage8c-action-parity
git pull --ff-only origin feature/herdr-stage8c-action-parity

cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs

$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
cargo run --locked -- .
```

Use a **disposable test repository on JaviPi**, not a project with uncommitted work you need to retain.

1. Open one local Windows agent and one JaviPi agent, each pointed at a different Git repository. Switch back and forth with the compact Machines rail. The Files/Changes/Branches panels must follow the active agent.
2. On JaviPi, right-click a modified file in **Changes**. Verify `Open file` opens its **Linux** file in a central pane. Verify `Stage`, `Unstage`, and the checkbox act on JaviPi only.
3. Use **Discard changes…** on a test file. Cancel must do nothing. Confirm once and verify the reviewed changes are discarded only in the remote repository. If another process modifies Git index/HEAD after the snapshot, the bridge should reject the stale operation.
4. Stage a changed file, enter a subject **and a multiline description**, press Commit then Confirm. Inspect `git log -1 --format=%B` on JaviPi and check both parts are preserved. Change either field between clicks to ensure the confirmation resets.
4a. With a clean staged index, click **Amend**. Existing subject/body must fill into blank fields; submit an amended subject and confirm. Verify `git log -1 --format=%B` shows the new message and a different HEAD. Check the next commit has Amend turned **off**.
4b. Stage another change; click **Write it**. Verify the remote Shell produces a conventional summary/body on JaviPi; the **Windows** Shell must not be invoked. Test failure from a stale index while generation runs.
4c. Test **Commit & Push** on a disposable repository with a safe configured HTTPS/SSH remote. Confirm twice; verify commit + push. Without a configured remote, it must fail **before** committing; if credentials/network fail after committing, the UI must refresh and report partial success.
5. Modify a disposable file and **Stash all changes** with an empty message: it should create a stash with `WIP (Gentle Shell)`. Non-empty messages should remain unchanged.
6. From **Branches** on JaviPi, click the Git History graph button in the footer. It must open a remote History pane tied to the selected agent (no Windows Git history).
7. Open a branch and History on Windows and verify the preexisting native behavior remains unchanged. Check the remote Profiles and Palette screens read their own source, without borrowing Windows settings.
8. Disconnect JaviPi during a read or mutation, reconnect through Herdr, and confirm remote editors, sessions and latest Git status remain correctly scoped.

## Remaining differences outside the 8C Git action batch

- Remote worktree creation is guarded and visible, but auto-opening a *new* Herdr agent at the returned worktree path is not yet wired. Local worktree creation can open a local agent automatically.
- Desktop-local Settings/Providers/Setup are deliberately bound to the Windows desktop, not misrepresented as remote Shell configuration. Profiles use remote configuration RPC when supported.
- Actual Windows/JaviPi interactive validation (including network credentials, reconnection and CLI generation on Pi4) is required before declaring full user-experience parity.

**Verification status:** JavaScript source and test-file syntax were checked; the full Rust toolchain and a live JaviPi/Windows desktop were not available in the implementation environment. GitHub Actions currently reports no workflow runs for this PR. Do **not** merge 8C into `main` until Cargo tests, Clippy, Node bridge tests and the interactive local/remote checks are green.
