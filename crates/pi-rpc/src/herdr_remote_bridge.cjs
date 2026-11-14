// Remote Pi RPC bridge injected by GPUI into Node.js through Herdr's
// terminal.run argv. No installation, npm install or Rust compilation
// is needed on JaviPi. The opt-in herdr_fs_write command can atomically
// replace an existing text file after checking its content revision.
"use strict";

const { spawn, execFile } = require("node:child_process");

const token = process.argv[1] || "";
if (!/^[a-zA-Z0-9]{1,48}$/.test(token)) {
  process.stderr.write("Herdr bridge: invalid session token\n");
  process.exit(2);
}

const prefix = "GS4A1|" + token + "|";
const STARTUP_ID = "gs4a-startup";
const MAX_FRAME = 96;
const MAX_LINE = 4096;
const MAX_JSONL = 8 * 1024 * 1024;
// Pi4 needs time to initialize installed extensions and optional memory hooks.
const STARTUP_MS = 60000;

function emit(kind, data) {
  const bytes = Buffer.isBuffer(data) ? data : Buffer.from(String(data), "utf8");
  if (bytes.length === 0) {
    process.stdout.write(prefix + kind + "|\n");
    return;
  }
  for (let i = 0; i < bytes.length; i += MAX_FRAME) {
    process.stdout.write(prefix + kind + "|" +
      bytes.subarray(i, i + MAX_FRAME).toString("base64") + "\n");
  }
}

let stopped = false;
let ready = false;
let stderrTail = "";
let startupRecords = 0;
let startupNonJson = 0;
let lastStartupType = "none";
const agentBin = process.env.GENTLE_SHELL_BIN || "gentle-shell";
const agent = spawn(agentBin, ["--mode", "rpc"], {
  cwd: process.cwd(),
  env: { ...process.env, GENTLE_SHELL_INTERACTIVE_HOST: "1" },
  stdio: ["pipe", "pipe", "pipe"],
  shell: false,
});

function fail(reason) {
  if (stopped) return;
  stopped = true;
  emit("E", "Remote Gentle Shell startup/transport failed: " + reason +
    (stderrTail ? " | " + stderrTail : ""));
  try { agent.kill(); } catch {}
}

const startupTimer = setTimeout(() => {
  if (!ready) fail(
    "Pi did not answer get_state within 60 seconds" +
    " (stdout_records=" + startupRecords +
    ", invalid_json=" + startupNonJson +
    ", last_type=" + lastStartupType + ")"
  );
}, STARTUP_MS);

agent.on("error", (error) => {
  fail("could not start " + agentBin + ": " + error.message);
});

agent.on("close", (code, signal) => {
  clearTimeout(startupTimer);
  process.stdin.pause();
  emit("X", String(Number.isInteger(code) ? code : 1));
  process.exitCode = Number.isInteger(code) ? code : 1;
});

// A Pi record can span arbitrary pipe chunks, but only bounded JSONL records
// are accepted. Startup acknowledgment is consumed by this bridge.
let piBuffer = Buffer.alloc(0);
agent.stdout.on("data", (chunk) => {
  if (stopped) return;
  piBuffer = Buffer.concat([piBuffer, chunk]);
  let cut;
  while ((cut = piBuffer.indexOf(10)) !== -1) {
    const line = piBuffer.subarray(0, cut + 1);
    piBuffer = piBuffer.subarray(cut + 1);
    if (line.length > MAX_JSONL) {
      fail("Pi JSONL record exceeds 8 MiB");
      return;
    }
    if (!ready) {
      startupRecords++;
      let response;
      try { response = JSON.parse(line.toString("utf8")); } catch {
        startupNonJson++;
      }
      if (response && typeof response.type === "string") {
        lastStartupType = response.type;
      }
      if (response && response.type === "response" &&
          response.id === STARTUP_ID) {
        if (response.success === true) {
          ready = true;
          clearTimeout(startupTimer);
          emit("R", Buffer.alloc(0));
        } else {
          fail("Pi get_state failed: " +
            (typeof response.error === "string" ? response.error : "unknown error"));
        }
        continue;
      }
    }
    emit("O", line);
  }
  if (piBuffer.length > MAX_JSONL) fail("Pi JSONL record exceeds 8 MiB");
});

agent.stderr.on("data", (chunk) => {
  if (stderrTail.length < 4096) {
    stderrTail += chunk.toString("utf8").slice(0, 4096 - stderrTail.length);
  }
  emit("E", chunk);
});


// Same-process remote filesystem RPC. These private commands are intercepted
// before Pi sees them: no visible bash/tool transcript, extra Herdr terminal
// or SSH. Writes require an explicit matching content revision.
const fs = require("node:fs/promises");
const { constants: fsConstants } = require("node:fs");
const { createHash, randomBytes } = require("node:crypto");
const path = require("node:path");
const { TextDecoder } = require("node:util");
const MAX_REMOTE_FILE = 128 * 1024;
const MAX_REMOTE_ENTRIES = 500;
const writeInFlight = new Set();

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function decodeText(bytes) {
  if (bytes.length > MAX_REMOTE_FILE || bytes.includes(0)) {
    throw new Error("Binary files and files over 128 KiB cannot be edited");
  }
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

function validRevision(revision) {
  return typeof revision === "string" && /^[a-f0-9]{64}$/.test(revision);
}

function validLinuxPath(value) {
  return typeof value === "string" && value.length > 0 &&
    value.length <= 2048 && value.startsWith("/") && !value.includes("\\") &&
    !/[\x00-\x1f\x7f]/.test(value) &&
    !value.split("/").some((part) => part === ".." || part === ".");
}

function within(root, candidate) {
  return candidate === root || (root === "/"
    ? candidate.startsWith("/") : candidate.startsWith(root + "/"));
}

async function safeRemotePath(root, candidate) {
  if (!validLinuxPath(root) || !validLinuxPath(candidate)) {
    throw new Error("Invalid absolute Linux path");
  }
  const actualRoot = await fs.realpath(root);
  const runtimeRoot = await fs.realpath(process.cwd());
  if (actualRoot !== runtimeRoot) {
    throw new Error("The requested root does not match this agent's working directory");
  }
  // Both lexical and resolved boundaries: reject .., traversals and
  // symlinks escaping the agent's original working directory.
  if (!within(path.posix.normalize(root), path.posix.normalize(candidate))) {
    throw new Error("The requested path is outside the agent's directory");
  }
  const actual = await fs.realpath(candidate);
  if (!within(actualRoot, actual)) {
    throw new Error("The requested path resolves outside the agent's directory");
  }
  const details = await fs.lstat(candidate);
  if (details.isSymbolicLink()) {
    throw new Error("Symlinks are not opened by the read-only remote browser");
  }
  return candidate;
}

// A constrained, explicit compare-before-replace write for existing text
// files only. The bridge serializes writes to the same path; other processes
// may still edit the file, so it checks both hash and inode again immediately
// before rename. No API can promise an OS-level atomic CAS against unrelated
// editors without a cooperative lock. A detected conflict always rejects.
async function remoteWrite(req) {
  // A remote agent launched at '/' can read/browse, but opting into writes
  // requires choosing an actual project directory as the agent root.
  if (await fs.realpath(process.cwd()) === "/") {
    throw new Error("Remote editing requires an agent started inside a project directory, not /");
  }
  const validated = await safeRemotePath(req.root, req.path);
  // Work only on the resolved, in-root location: aliases through allowed
  // parent symlinks cannot redirect or bypass the per-file write guard.
  const selected = await fs.realpath(validated);
  if (typeof req.content !== "string" || !validRevision(req.expectedRevision)) {
    throw new Error("Write requires text and a matching SHA-256 revision");
  }
  const nextBytes = Buffer.from(req.content, "utf8");
  if (nextBytes.toString("utf8") !== req.content ||
      nextBytes.length > MAX_REMOTE_FILE || nextBytes.includes(0)) {
    throw new Error("Only valid UTF-8 text up to 128 KiB can be saved");
  }
  if (writeInFlight.has(selected)) {
    throw new Error("Another save is already running for this file");
  }
  writeInFlight.add(selected);
  let temp = null;
  try {
    // O_NOFOLLOW prevents a replaced final symlink from being followed.
    // O_NONBLOCK avoids waiting forever if the file becomes a FIFO.
    const handle = await fs.open(selected,
      fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW | fsConstants.O_NONBLOCK);
    let stat;
    let original;
    try {
      stat = await handle.stat();
      if (!stat.isFile() || stat.nlink !== 1 || stat.size > MAX_REMOTE_FILE) {
        throw new Error("Only existing regular, non-linked text files up to 128 KiB can be saved");
      }
      if (typeof process.getuid === "function" && stat.uid !== process.getuid()) {
        throw new Error("Only files owned by the remote agent can be edited");
      }
      original = await handle.readFile();
    } finally {
      await handle.close();
    }
    decodeText(original);
    if (sha256(original) !== req.expectedRevision) {
      throw new Error("Conflict: the remote file changed since it was opened. Your draft was not saved");
    }
    // Place a private temporary file in the same directory for atomic rename
    // on the same filesystem. Never create user-visible destination files.
    const directory = path.posix.dirname(selected);
    temp = path.posix.join(directory,
      ".herdr-save-" + randomBytes(16).toString("hex") + ".tmp");
    const output = await fs.open(temp,
      fsConstants.O_WRONLY | fsConstants.O_CREAT |
      fsConstants.O_EXCL | fsConstants.O_NOFOLLOW, stat.mode & 0o777);
    try {
      await output.writeFile(nextBytes);
      await output.chmod(stat.mode & 0o777);
      await output.sync();
    } finally {
      await output.close();
    }
    // Revalidate after staging: file identity, bytes and the original root.
    await safeRemotePath(req.root, validated);
    const latest = await fs.lstat(selected);
    if (!latest.isFile() || latest.isSymbolicLink() ||
        latest.dev !== stat.dev || latest.ino !== stat.ino ||
        latest.size !== stat.size || latest.mtimeMs !== stat.mtimeMs ||
        latest.ctimeMs !== stat.ctimeMs) {
      throw new Error("Conflict: file identity or metadata changed. Your draft was not saved");
    }
    const now = await fs.readFile(selected);
    if (sha256(now) !== req.expectedRevision) {
      throw new Error("Conflict: remote content changed during save. Your draft was not saved");
    }
    await fs.rename(temp, selected);
    temp = null;
    return { path: selected, size: nextBytes.length, revision: sha256(nextBytes) };
  } finally {
    if (temp) {
      try { await fs.unlink(temp); } catch {}
    }
    writeInFlight.delete(selected);
  }
}

async function remoteFilesRequest(req) {
  const command = req.type;
  const response = (success, data, error) => emit("O", JSON.stringify({
    type: "response", command, id: req.id, success,
    ...(success ? { data } : { error }),
  }) + "\n");
  try {
    if (typeof req.id !== "string" || req.id.length > 128) {
      throw new Error("Missing request identifier");
    }
    if (!["herdr_fs_list", "herdr_fs_read", "herdr_fs_write"].includes(command)) {
      throw new Error("Unsupported filesystem operation");
    }
    if (command === "herdr_fs_write") {
      response(true, await remoteWrite(req));
      return;
    }
    const selected = await safeRemotePath(req.root, req.path);
    if (command === "herdr_fs_list") {
      const entries = await fs.readdir(selected, { withFileTypes: true });
      const sorted = entries
        .map((entry) => ({
          name: entry.name,
          kind: entry.isDirectory() ? "directory" :
            entry.isSymbolicLink() ? "symlink" : entry.isFile() ? "file" : "other",
        }))
        .sort((a, b) => (a.kind === "directory" ? 0 : 1) -
                         (b.kind === "directory" ? 0 : 1) ||
                         a.name.localeCompare(b.name));
      response(true, {
        path: selected, entries: sorted.slice(0, MAX_REMOTE_ENTRIES),
        truncated: sorted.length > MAX_REMOTE_ENTRIES,
      });
    } else {
      const stat = await fs.stat(selected);
      if (!stat.isFile()) throw new Error("Not a regular file");
      if (stat.size > MAX_REMOTE_FILE) {
        throw new Error("File exceeds the 128 KiB read-only preview limit");
      }
      const bytes = await fs.readFile(selected);
      if (bytes.length > MAX_REMOTE_FILE || bytes.includes(0)) {
        throw new Error("Binary files and files over 128 KiB cannot be previewed");
      }
      const content = decodeText(bytes);
      response(true, {
        path: selected, content, size: bytes.length,
        revision: sha256(bytes),
      });
    }
  } catch (error) {
    response(false, null, error && error.message ? error.message : "Filesystem query failed");
  }
}

// Remote Git is scoped to the currently selected agent's Linux repository.
// 5C reads remain unchanged; 5D mutations have separate, explicit commands.
// Never run Git aliases, a shell, local Windows Git or network Git actions.
const MAX_GIT_LIST = 200;
const MAX_GIT_BRANCHES = 120;
const MAX_GIT_DIFF = 96 * 1024;

function strictUtf8(bytes) {
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

function runGit(directory, args, allowedDiffExit = false) {
  return new Promise((resolve, reject) => {
    execFile("git", [
      "--no-optional-locks",
      "-c", "core.fsmonitor=false",
      "-c", "diff.external=",
      "-c", "diff.trustExitCode=false",
      "-C", directory, ...args,
    ], {
      shell: false,
      timeout: 8000,
      maxBuffer: 384 * 1024,
      encoding: "buffer",
      env: {
        ...process.env,
        GIT_PAGER: "cat",
        GIT_EXTERNAL_DIFF: "0",
        GIT_TERMINAL_PROMPT: "0",
      },
    }, (error, stdout, stderr) => {
      if (error && !(allowedDiffExit && error.code === 1 &&
            !error.killed && !error.signal)) {
        const detail = Buffer.from(stderr || "").toString("utf8").trim() ||
          (error && error.message ? error.message : "Unknown Git error");
        reject(new Error("Remote Git query failed: " + detail.slice(0, 300)));
      } else {
        resolve(Buffer.from(stdout));
      }
    });
  });
}

// Stream large status/branch listings instead of buffering the full Git
// output. A busy repository may have millions of untracked files: collect
// at most N complete records plus one sentinel, then stop Git. This is not
// a transport failure; it is an explicitly marked partial snapshot.
// Mutations use mandatory Git locks and never run hooks or launch editors.
// Git receives fixed verbs and validated arguments, not arbitrary commands.
function runGitMutation(directory, args) {
  return new Promise((resolve, reject) => {
    execFile("git", [
      "-c", "core.fsmonitor=false",
      "-c", "core.hooksPath=/dev/null",
      "-c", "commit.gpgsign=false",
      "-C", directory, ...args,
    ], {
      shell: false, timeout: 30000, maxBuffer: 128 * 1024,
      encoding: "buffer",
      env: { ...process.env, GIT_PAGER: "cat",
        GIT_TERMINAL_PROMPT: "0", GIT_EDITOR: "true",
        GIT_EXTERNAL_DIFF: "0" },
    }, (error, stdout, stderr) => {
      if (error) {
        const detail = Buffer.from(stderr || "").toString("utf8").trim() ||
          error.message || "Git mutation failed";
        reject(new Error("Remote Git write failed: " + detail.slice(0, 300)));
      } else {
        resolve(Buffer.from(stdout));
      }
    });
  });
}

function runGitListing(directory, args, separator, maxRecords) {
  return new Promise((resolve, reject) => {
    const child = spawn("git", [
      "--no-optional-locks",
      "-c", "core.fsmonitor=false",
      "-c", "diff.external=",
      "-c", "diff.trustExitCode=false",
      "-C", directory, ...args,
    ], {
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        GIT_PAGER: "cat",
        GIT_EXTERNAL_DIFF: "0",
        GIT_TERMINAL_PROMPT: "0",
      },
    });
    const MAX_RECORD_BYTES = 4096;
    const MAX_STDERR = 1024;
    const records = [];
    let pending = Buffer.alloc(0);
    let stderr = "";
    let truncated = false;
    let failure = null;
    let timeoutExpired = false;
    const timer = setTimeout(() => {
      timeoutExpired = true;
      child.kill("SIGKILL");
    }, 15000);
    child.stdout.on("data", (chunk) => {
      if (truncated || failure || timeoutExpired) return;
      pending = Buffer.concat([pending, chunk]);
      let cut;
      while ((cut = pending.indexOf(separator)) !== -1) {
        const record = pending.subarray(0, cut);
        pending = pending.subarray(cut + 1);
        if (record.length > MAX_RECORD_BYTES) {
          failure = "Remote Git record exceeds the 4096-byte safety limit";
          child.kill("SIGKILL");
          return;
        }
        // The extra record tells us whether more than maxRecords exist.
        if (records.length === maxRecords) {
          truncated = true;
          child.kill("SIGTERM");
          return;
        }
        records.push(Buffer.concat([record, Buffer.from([separator])]));
      }
      if (pending.length > MAX_RECORD_BYTES) {
        failure = "Remote Git record exceeds the 4096-byte safety limit";
        child.kill("SIGKILL");
      }
    });
    child.stderr.on("data", (chunk) => {
      if (stderr.length < MAX_STDERR) {
        stderr += chunk.toString("utf8").slice(0, MAX_STDERR - stderr.length);
      }
    });
    child.on("error", (error) => {
      failure = error.message;
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (failure) {
        reject(new Error("Remote Git query failed: " + failure));
      } else if (timeoutExpired) {
        reject(new Error("Remote Git query timed out after 8 seconds"));
      } else if (!truncated && code !== 0) {
        reject(new Error("Remote Git query failed: " +
          (stderr.trim() || "Git exit code " + code).slice(0, 300)));
      } else if (!truncated && pending.length) {
        reject(new Error("Remote Git query returned an incomplete record"));
      } else {
        resolve({ output: Buffer.concat(records), truncated });
      }
    });
  });
}

async function gitRepository(root) {
  const selected = await safeRemotePath(root, root);
  const actualRoot = await fs.realpath(selected);
  const output = await runGit(actualRoot, ["rev-parse", "--show-toplevel"]);
  const declared = strictUtf8(output).trim();
  if (!validLinuxPath(declared)) {
    throw new Error("Invalid Git repository location");
  }
  const toplevel = await fs.realpath(declared);
  // Never query a parent repository outside the current agent's project.
  if (!within(actualRoot, toplevel)) {
    throw new Error("Git repository is outside this agent's directory. Start the agent at the repository root");
  }
  return { toplevel, actualRoot };
}

function safeGitRelative(name) {
  return typeof name === "string" && name.length > 0 &&
    name.length <= 2048 && !name.startsWith("/") &&
    !name.includes("\\") && !name.includes(":") &&
    !/[\x00-\x1f\x7f]/.test(name) &&
    !name.split("/").some((part) => part === "." || part === "..");
}

function parseGitEntries(output) {
  const entries = [];
  const segments = output.toString("utf8").split("\0");
  for (const segment of segments) {
    if (!segment) continue;
    if (segment.length < 4 || segment[2] !== " ") {
      throw new Error("Invalid Git status record");
    }
    const name = segment.slice(3);
    if (!safeGitRelative(name)) {
      throw new Error("Invalid Git status path");
    }
    if (entries.length >= MAX_GIT_LIST) break;
    entries.push({ path: name, index: segment[0], worktree: segment[1] });
  }
  return { entries, truncated: segments.filter(Boolean).length > entries.length };
}

function parseGitBranches(output) {
  const rows = strictUtf8(output).split("\n").filter(Boolean);
  const branches = [];
  for (const row of rows.slice(0, MAX_GIT_BRANCHES)) {
    const [name, mark, fullRef] = row.split("\0");
    if (!name || name.length > 512 || /[\x00-\x1f\x7f]/.test(name)) {
      throw new Error("Invalid Git branch");
    }
    branches.push({ name, current: mark === "*",
      local: typeof fullRef === "string" && fullRef.startsWith("refs/heads/") });
  }
  return { branches, truncated: rows.length > branches.length };
}

async function gitRefState(toplevel) {
  const headOid = await runGit(toplevel, ["rev-parse", "--verify", "-q", "HEAD"])
    .then((bytes) => strictUtf8(bytes).trim()).catch(() => null);
  const currentBranch = await runGit(toplevel, [
    "symbolic-ref", "--quiet", "--short", "HEAD",
  ]).then((bytes) => strictUtf8(bytes).trim()).catch(() => null);
  return { headOid, currentBranch };
}

async function remoteGit(req) {
  const { toplevel } = await gitRepository(req.root);
  if (req.type === "herdr_git_snapshot") {
    const [status, branches, index] = await Promise.all([
      runGitListing(toplevel, ["status", "--porcelain=v1", "-z",
        "--no-renames", "--untracked-files=all", "--ignore-submodules=all"],
        0, MAX_GIT_LIST),
      runGitListing(toplevel, ["branch", "--all", "--list",
        "--format=%(refname:short)%00%(HEAD)%00%(refname)"],
        10, MAX_GIT_BRANCHES),
      runGitListing(toplevel, ["diff", "--cached", "--raw",
        "-z", "--no-renames"], 0, MAX_GIT_LIST * 2),
    ]);
    const files = parseGitEntries(status.output);
    const refs = parseGitBranches(branches.output);
    const { headOid, currentBranch } = await gitRefState(toplevel);
    return {
      toplevel, headOid, currentBranch, files: files.entries,
      indexToken: sha256(index.output), indexTruncated: index.truncated,
      filesTruncated: files.truncated || status.truncated,
      branches: refs.branches,
      branchesTruncated: refs.truncated || branches.truncated,
      readOnly: true, writesEnabled: true,
    };
  }
  if (req.type !== "herdr_git_diff" || !validLinuxPath(req.path)) {
    throw new Error("Unsupported remote Git request or path");
  }
  const target = path.posix.normalize(req.path);
  if (!within(toplevel, target) || target === toplevel) {
    throw new Error("Git path is outside the selected repository");
  }
  const relative = path.posix.relative(toplevel, target);
  if (!safeGitRelative(relative)) {
    throw new Error("Unsafe Git diff path");
  }
  // Do not expose arbitrary files through Git diff. Only paths currently
  // reported as changed may be inspected.
  // Scope the membership check to one literal path. The full repository
  // status could exceed maxBuffer even when this one file has a tiny diff.
  const status = parseGitEntries(await runGit(toplevel, [
    "status", "--porcelain=v1", "-z", "--no-renames",
    "--untracked-files=all", "--ignore-submodules=all",
    "--", ":(literal)" + relative,
  ]));
  const entry = status.entries.find((x) => x.path === relative);
  if (!entry) {
    throw new Error("Git path is not a changed file (or status list was truncated)");
  }
  let output;
  if (entry.index === "?" && entry.worktree === "?") {
    // O_NOFOLLOW style path validation applies before diffing untracked
    // files; this blocks symlinks escaping the remote agent's root.
    await safeRemotePath(req.root, target);
    const st = await fs.stat(target);
    if (!st.isFile() || st.size > MAX_REMOTE_FILE) {
      throw new Error("Untracked diff requires a regular file under 128 KiB");
    }
    output = await runGit(toplevel, [
      "diff", "--no-ext-diff", "--no-textconv", "--no-index",
      "--", "/dev/null", target,
    ], true);
  } else {
    // HEAD combines index and working-tree changes in one patch.
    // Unborn repositories fall back to the staged diff first.
    try {
      output = await runGit(toplevel, [
        "diff", "--no-ext-diff", "--no-textconv", "--no-renames",
        "HEAD", "--", relative,
      ]);
    } catch {
      output = await runGit(toplevel, [
        "diff", "--cached", "--no-ext-diff", "--no-textconv",
        "--no-renames", "--", relative,
      ]);
      if (!output.length) {
        output = await runGit(toplevel, [
          "diff", "--no-ext-diff", "--no-textconv", "--no-renames",
          "--", relative,
        ]);
      }
    }
  }
  if (output.length > MAX_GIT_DIFF) {
    throw new Error("Diff exceeds the 96 KiB remote preview limit");
  }
  return {
    toplevel, path: target, patch: strictUtf8(output), readOnly: true,
  };
}


// Only explicit mutations below may change a repo. Each request names the
// exact repo and the HEAD/branch displayed by the UI. No stage-all or
// unstage-all, no discard, reset, push, pull, fetch or forced checkout.
const gitWriteInFlight = new Set();
const GIT_MUTATIONS = [
  "herdr_git_stage", "herdr_git_unstage", "herdr_git_commit",
  "herdr_git_create_branch", "herdr_git_switch_branch",
];

async function checkGitIndexRequest(req, toplevel) {
  if (typeof req.expectedIndexToken !== "string" ||
      !/^[a-f0-9]{64}$/.test(req.expectedIndexToken)) {
    throw new Error("Missing remote Git index revision; refresh before writing");
  }
  const index = await runGitListing(toplevel, [
    "diff", "--cached", "--raw", "-z", "--no-renames",
  ], 0, MAX_GIT_LIST * 2);
  if (index.truncated || sha256(index.output) !== req.expectedIndexToken) {
    throw new Error("Conflict: staged Git index changed. Refresh before writing");
  }
  return index.output.length > 0;
}

function validateGitRefRequest(req, toplevel) {
  if (req.toplevel !== toplevel || toplevel === "/") {
    throw new Error("Remote Git write requires the selected repository root");
  }
  if (!Object.hasOwn(req, "expectedHead") ||
      !Object.hasOwn(req, "expectedBranch") ||
      !(req.expectedHead === null ||
        (typeof req.expectedHead === "string" &&
          /^[a-f0-9]{40,64}$/.test(req.expectedHead))) ||
      !(req.expectedBranch === null ||
        (typeof req.expectedBranch === "string" &&
          req.expectedBranch.length > 0 && req.expectedBranch.length <= 512))) {
    throw new Error("Remote Git write requires the expected HEAD and branch");
  }
}

async function checkGitRefRequest(req, toplevel) {
  const actual = await gitRefState(toplevel);
  if (actual.headOid !== req.expectedHead ||
      actual.currentBranch !== req.expectedBranch) {
    throw new Error("Conflict: Git HEAD or branch changed. Refresh before writing");
  }
}

async function selectedGitEntry(req, toplevel) {
  if (!validLinuxPath(req.path)) {
    throw new Error("Expected an absolute remote Git file path");
  }
  const target = path.posix.normalize(req.path);
  if (target === toplevel || !within(toplevel, target)) {
    throw new Error("Git file is outside the selected repository");
  }
  const relative = path.posix.relative(toplevel, target);
  if (!safeGitRelative(relative)) {
    throw new Error("Invalid Git file name");
  }
  // Never stage a symlink, an external file, or a directory. A deleted
  // tracked file is allowed only if its parent still resolves within root.
  const parent = await fs.realpath(path.posix.dirname(target));
  if (!within(toplevel, parent)) {
    throw new Error("Git file parent resolves outside the repository");
  }
  let stat = null;
  try { stat = await fs.lstat(target); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  if (stat && (!stat.isFile() || stat.isSymbolicLink())) {
    throw new Error("Only regular files and tracked deletions may be staged");
  }
  if (stat && !within(toplevel, await fs.realpath(target))) {
    throw new Error("Git file resolves outside the repository");
  }
  if (typeof req.expectedIndex !== "string" || req.expectedIndex.length !== 1 ||
      typeof req.expectedWorktree !== "string" ||
      req.expectedWorktree.length !== 1) {
    throw new Error("Missing expected Git file state");
  }
  const output = await runGit(toplevel, [
    "status", "--porcelain=v1", "-z", "--no-renames",
    "--untracked-files=all", "--ignore-submodules=all",
    "--", ":(literal)" + relative,
  ]);
  const entries = parseGitEntries(output).entries;
  if (entries.length !== 1 || entries[0].path !== relative ||
      entries[0].index !== req.expectedIndex ||
      entries[0].worktree !== req.expectedWorktree) {
    throw new Error("Conflict: Git file changed. Refresh before writing");
  }
  return relative;
}

function validatedBranchName(name) {
  if (typeof name !== "string" || name.length > 120 ||
      !/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(name)) {
    throw new Error("Invalid local branch name");
  }
  return name;
}

async function remoteGitWrite(req) {
  const { toplevel } = await gitRepository(req.root);
  validateGitRefRequest(req, toplevel);
  if (gitWriteInFlight.has(toplevel)) {
    throw new Error("Another remote Git write is already in progress");
  }
  gitWriteInFlight.add(toplevel);
  try {
    await checkGitRefRequest(req, toplevel);
    const hasStaged = await checkGitIndexRequest(req, toplevel);
    if (req.type === "herdr_git_stage" || req.type === "herdr_git_unstage") {
      const relative = await selectedGitEntry(req, toplevel);
      const literal = ":(literal)" + relative;
      if (req.type === "herdr_git_stage") {
        if (req.expectedWorktree === " " && req.expectedIndex !== "?") {
          throw new Error("No unstaged change for the selected file");
        }
        await runGitMutation(toplevel, ["add", "-A", "--", literal]);
      } else {
        if (req.expectedIndex === " " || req.expectedIndex === "?") {
          throw new Error("No staged change for the selected file");
        }
        if (req.expectedHead === null) {
          await runGitMutation(toplevel, ["rm", "--cached", "-q", "--", literal]);
        } else {
          await runGitMutation(toplevel, ["restore", "--staged", "--", literal]);
        }
      }
    } else if (req.type === "herdr_git_commit") {
      if (!req.expectedBranch) {
        throw new Error("Cannot commit from a detached HEAD");
      }
      const summary = req.summary;
      if (typeof summary !== "string" || summary.trim() !== summary ||
          summary.length < 1 || summary.length > 120 ||
          /[\x00-\x1f\x7f]/.test(summary)) {
        throw new Error("Commit summary must be 1-120 characters on one line");
      }
      const message = req.description === undefined ? "" : req.description;
      if (typeof message !== "string" || message.length > 4000 ||
          message.includes("\0")) {
        throw new Error("Invalid commit description");
      }
      if (!hasStaged) {
        throw new Error("No staged changes to commit");
      }
      const args = ["commit", "--no-verify", "--no-gpg-sign", "-m", summary];
      if (message.trim()) args.push("-m", message.trim());
      await runGitMutation(toplevel, args);
    } else if (req.type === "herdr_git_create_branch" ||
               req.type === "herdr_git_switch_branch") {
      if (!req.expectedBranch || !req.expectedHead) {
        throw new Error("Branch operations need an existing branch and commit");
      }
      const branch = validatedBranchName(req.branch);
      await runGit(toplevel, ["check-ref-format", "--branch", branch]);
      if (req.type === "herdr_git_create_branch") {
        await runGitMutation(toplevel, ["branch", branch]);
      } else {
        // Refuse every switch when dirty, including untracked files.
        const dirty = await runGitListing(toplevel, [
          "status", "--porcelain=v1", "-z",
          "--untracked-files=all",
        ], 0, 1);
        if (dirty.output.length || dirty.truncated) {
          throw new Error("Switch requires a clean index and worktree");
        }
        await runGit(toplevel, [
          "show-ref", "--verify", "--quiet", "refs/heads/" + branch,
        ]);
        await runGitMutation(toplevel, ["switch", "--no-guess", branch]);
      }
    } else {
      throw new Error("Unsupported remote Git write");
    }
    return { toplevel, operation: req.type, ...await gitRefState(toplevel) };
  } finally {
    gitWriteInFlight.delete(toplevel);
  }
}

async function remoteGitRequest(req) {
  const command = req.type;
  const response = (success, data, error) => emit("O", JSON.stringify({
    type: "response", command, id: req.id, success,
    ...(success ? { data } : { error }),
  }) + "\n");
  try {
    if (typeof req.id !== "string" || req.id.length > 128) {
      throw new Error("Missing request identifier");
    }
    if (!["herdr_git_snapshot", "herdr_git_diff",
          ...GIT_MUTATIONS].includes(command)) {
      throw new Error("Unsupported remote Git operation");
    }
    response(true, GIT_MUTATIONS.includes(command) ?
      await remoteGitWrite(req) : await remoteGit(req));
  } catch (error) {
    response(false, null,
      error && error.message ? error.message : "Remote Git query failed");
  }
}

// The PTY is line-buffered: GPUI submits each encoded input frame with CR.
// Accept CR, LF and CRLF, echo/control-character noise and stream splits.
let rpcInput = [];
let inputBuffer = [];
let discard = false;
function inputLine() {
  if (discard) return;
  const line = Buffer.from(inputBuffer).toString("ascii");
  const start = line.indexOf(prefix);
  if (start < 0) return;
  const tail = line.slice(start + prefix.length);
  if (!tail.startsWith("I|")) return;
  const encoded = tail.slice(2);
  if (!/^(?:[a-zA-Z0-9+/]{4})*(?:[a-zA-Z0-9+/]{2}==|[a-zA-Z0-9+/]{3}=)?$/.test(encoded)) return;
  const payload = Buffer.from(encoded, "base64");
  if (payload.length > MAX_FRAME || stopped) return;
  if (!agent.stdin.destroyed && !agent.stdin.writableEnded) {
    // A serialized Pi RPC request can span many framed chunks. Assemble
    // complete newline-delimited JSON before choosing the private fs
    // handler or forwarding the original request unchanged to Pi.
    for (const byte of payload) {
      if (rpcInput.length >= MAX_JSONL) {
        rpcInput = [];
        fail("Pi RPC input record exceeds 8 MiB");
        return;
      }
      rpcInput.push(byte);
      if (byte === 10) {
        const requestLine = Buffer.from(rpcInput);
        rpcInput = [];
        let request = null;
        try { request = JSON.parse(requestLine.toString("utf8")); } catch {}
        if (request && (request.type === "herdr_fs_list" ||
                        request.type === "herdr_fs_read" ||
                        request.type === "herdr_fs_write")) {
          void remoteFilesRequest(request);
        } else if (request && (request.type === "herdr_git_snapshot" ||
                               request.type === "herdr_git_diff" ||
                               GIT_MUTATIONS.includes(request.type))) {
          void remoteGitRequest(request);
        } else {
          agent.stdin.write(requestLine);
        }
      }
    }
  }
}

process.stdin.on("data", (data) => {
  for (const byte of data) {
    if (byte === 10 || byte === 13) {
      inputLine();
      inputBuffer = [];
      discard = false;
    } else if (!discard) {
      if (inputBuffer.length < MAX_LINE) {
        inputBuffer.push(byte);
      } else {
        inputBuffer = [];
        discard = true;
      }
    }
  }
});
process.stdin.on("end", () => fail("Herdr remote terminal stdin closed"));
process.stdin.on("error", (e) => fail("Herdr remote terminal stdin error: " + e.message));
agent.stdin.on("error", (e) => fail("Pi RPC stdin error: " + e.message));
process.stdin.resume();

// Start the Pi child and verify RPC before claiming READY to the Windows UI.
agent.stdin.write(JSON.stringify({ type: "get_state", id: STARTUP_ID }) + "\n");

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    if (stopped) return;
    stopped = true;
    try { agent.kill(signal); } catch {}
    process.stdin.pause();
  });
}
