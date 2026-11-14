// Node's built-in test runner validates the zero-install remote JS bridge.
// No Pi, Herdr service, network, npm packages or Rust compiler needed.
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { EventEmitter } = require("node:events");
const { test } = require("node:test");

const source = fs.readFileSync(
  path.join(__dirname, "../src/herdr_remote_bridge.cjs"), "utf8"
);

function harness(cwd = "/srv/workspaces/JaviPi") {
  const stdout = [];
  const childWrites = [];
  let killed = false;
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.stdin = new EventEmitter();
  child.stdin.destroyed = false;
  child.stdin.writableEnded = false;
  child.stdin.write = (data) => childWrites.push(Buffer.from(data));
  child.kill = () => { killed = true; };
  const input = new EventEmitter();
  input.resume = () => {};
  input.pause = () => {};
  const calls = [];
  const timers = [];
  const fakeProcess = {
    argv: ["node", "t123"],
    env: { PATH: "/usr/bin", HOME: "/home/dietpi" },
    cwd: () => cwd,
    getuid: typeof process.getuid === "function" ? () => process.getuid() : undefined,
    stdout: { write: (value) => stdout.push(value) },
    stderr: { write: () => {} },
    stdin: input,
    on: () => {},
    exit: (code) => { throw Error("unexpected exit: " + code); },
  };
  const requireMock = (name) => {
    if (name === "node:child_process") {
      return {
        spawn: (...args) => {
          if (args[0] === "git") {
            return require("node:child_process").spawn(...args);
          }
          calls.push(args);
          return child;
        },
        // Git queries spawn only the git executable in real OS tests;
        // the agent remains a fake process and receives no Git RPC.
        execFile: (...args) => require("node:child_process").execFile(...args),
      };
    }
    // Remote Files uses built-in fs and SHA-256 for opt-in revision-checked
    // writes; no I/O occurs until the matching private command arrives.
    assert.ok(["node:fs/promises", "node:fs", "node:crypto",
               "node:path", "node:util"].includes(name));
    return require(name);
  };
  const execute = new Function(
    "require", "process", "Buffer", "setTimeout", "clearTimeout", source
  );
  execute(requireMock, fakeProcess, Buffer, (callback, milliseconds) => {
    timers.push({ callback, milliseconds });
    return timers.length;
  }, () => {});
  return {
    stdout,
    childWrites,
    child,
    input,
    calls,
    timers,
    fireStartupTimeout: () => timers[0].callback(),
    get killed() { return killed; },
  };
}

function readFrames(items) {
  return items.join("").split("\n").filter(Boolean).map((line) => {
    const marker = "GS4A1|t123|";
    assert.ok(line.startsWith(marker), "unexpected unframed output: " + line);
    const rest = line.slice(marker.length);
    const index = rest.indexOf("|");
    return {
      kind: rest.slice(0, index),
      payload: Buffer.from(rest.slice(index + 1), "base64"),
    };
  });
}

test("starts Gentle Shell RPC in remote cwd using remote environment", () => {
  const h = harness();
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0][0], "gentle-shell");
  assert.deepEqual(h.calls[0][1], ["--mode", "rpc"]);
  assert.equal(h.calls[0][2].cwd, "/srv/workspaces/JaviPi");
  assert.equal(h.calls[0][2].env.HOME, "/home/dietpi");
  assert.equal(h.calls[0][2].env.GENTLE_SHELL_INTERACTIVE_HOST, "1");
  assert.deepEqual(h.childWrites[0].toString(), 
    '{"type":"get_state","id":"gs4a-startup"}\n');
});

test("only declares READY after valid Pi get_state and forwards JSONL", () => {
  const h = harness();
  h.child.stdout.emit("data", Buffer.from(
    '{"type":"message_update","text":"starting"}\n' +
    '{"type":"response","id":"gs4a-startup","success":true,"data":{}}\n'
  ));
  const frames = readFrames(h.stdout);
  assert.deepEqual(frames.map((frame) => frame.kind), ["O", "R"]);
  const before = h.childWrites.length;
  const command = '{"type":"get_available_models","id":"gs-1"}\n';
  const encoded = Buffer.from(command).toString("base64");
  const line = "noise GS4A1|t123|I|" + encoded + "\r\n";
  h.input.emit("data", Buffer.from(line.slice(0, 18)));
  h.input.emit("data", Buffer.from(line.slice(18)));
  assert.equal(h.childWrites.length, before + 1);
  assert.equal(h.childWrites.at(-1).toString(), command);
  const reply = '{"type":"response","id":"gs-1","success":true,"data":{"models":[]}}\n';
  h.child.stdout.emit("data", Buffer.from(reply));
  const records = Buffer.concat(readFrames(h.stdout).filter((frame) =>
    frame.kind === "O").map((frame) => frame.payload)).toString();
  assert.ok(records.includes(reply));
});

test("failed Pi probe does not announce READY and terminates child", () => {
  const h = harness();
  h.child.stderr.emit("data", Buffer.from("extension failed\n"));
  h.child.stdout.emit("data", Buffer.from(
    '{"type":"response","id":"gs4a-startup","success":false,"error":"startup error"}\n'
  ));
  assert.equal(h.killed, true);
  assert.equal(readFrames(h.stdout).some((f) => f.kind === "R"), false);
  assert.ok(readFrames(h.stdout).some((f) =>
    f.kind === "E" && f.payload.toString().includes("startup error")));
});

test("ignores malformed, foreign-token and oversized input lines", () => {
  const h = harness();
  const before = h.childWrites.length;
  h.input.emit("data", Buffer.from("GS4A1|foreign|I|YQ==\r"));
  h.input.emit("data", Buffer.from("GS4A1|t123|I|!!invalid!!\r"));
  h.input.emit("data", Buffer.from("z".repeat(6000) + "\r"));
  assert.equal(h.childWrites.length, before);
});

test("Pi4 slow-start watchdog allows 60 s and reports missing RPC output", () => {
  const h = harness();
  assert.equal(h.timers[0].milliseconds, 60000);
  h.fireStartupTimeout();
  assert.equal(h.killed, true);
  const errors = Buffer.concat(readFrames(h.stdout).filter((f) =>
    f.kind === "E").map((f) => f.payload)).toString();
  assert.match(errors, /get_state within 60 seconds/);
  assert.match(errors, /stdout_records=0/);
});

test("startup timeout distinguishes no RPC from unexpected stdout records", () => {
  const h = harness();
  h.child.stdout.emit("data", Buffer.from(
    "unexpected cli banner\n" +
    '{"type":"agent_start"}\n'
  ));
  h.fireStartupTimeout();
  assert.equal(h.killed, true);
  const errors = Buffer.concat(readFrames(h.stdout).filter((f) =>
    f.kind === "E").map((f) => f.payload)).toString();
  assert.match(errors, /stdout_records=2/);
  assert.match(errors, /invalid_json=1/);
  assert.match(errors, /last_type=agent_start/);
});

function rpcInput(h, value) {
  const bytes = Buffer.from(JSON.stringify(value) + "\n");
  for (let i = 0; i < bytes.length; i += 96) {
    const chunk = bytes.subarray(i, i + 96).toString("base64");
    h.input.emit("data", Buffer.from("GS4A1|t123|I|" + chunk + "\r"));
  }
}

async function fsReply(h, id) {
  // Busy ARM64 disks may need more than 1 second to scan a large worktree.
  for (let attempt = 0; attempt < 2000; attempt++) {
    const stream = Buffer.concat(readFrames(h.stdout)
      .filter((frame) => frame.kind === "O")
      .map((frame) => frame.payload)).toString();
    const records = stream.split("\n").filter(Boolean);
    const found = records.map((line) => JSON.parse(line))
      .find((record) => record.id === id);
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error("Remote FS response timed out");
}

test("read-only remote file operations use existing bridge and never call Pi bash", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-remote-tree-"));
  try {
    fs.mkdirSync(path.join(root, "src"));
    fs.writeFileSync(path.join(root, "src", "hello.txt"), "hola desde javipi\n");
    fs.writeFileSync(path.join(root, "blob.dat"), Buffer.from([0, 1, 2]));
    const h = harness(root);
    const initialCalls = h.childWrites.length;

    rpcInput(h, { type: "herdr_fs_list", id: "fs-1", root, path: root });
    const listing = await fsReply(h, "fs-1");
    assert.equal(listing.success, true);
    assert.ok(listing.data.entries.some((x) => x.name === "src" && x.kind === "directory"));
    assert.equal(h.childWrites.length, initialCalls, "fs query must not go to Pi RPC");

    rpcInput(h, { type: "herdr_fs_read", id: "fs-2",
      root, path: path.join(root, "src", "hello.txt") });
    const preview = await fsReply(h, "fs-2");
    assert.equal(preview.success, true);
    assert.equal(preview.data.content, "hola desde javipi\n");
    assert.equal(h.childWrites.length, initialCalls, "preview must not invoke shell");

    rpcInput(h, { type: "herdr_fs_read", id: "fs-3",
      root, path: path.join(root, "blob.dat") });
    assert.equal((await fsReply(h, "fs-3")).success, false);

    rpcInput(h, { type: "herdr_fs_list", id: "fs-4",
      root, path: path.dirname(root) });
    assert.equal((await fsReply(h, "fs-4")).success, false);
    assert.equal(h.childWrites.length, initialCalls);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("read-only remote browser rejects invalid root and Windows paths", async () => {
  const h = harness();
  const before = h.childWrites.length;
  rpcInput(h, { type: "herdr_fs_list", id: "fs-bad1",
    root: "C:\\Users\\wrong", path: "/srv/workspaces/JaviPi" });
  assert.equal((await fsReply(h, "fs-bad1")).success, false);
  rpcInput(h, { type: "herdr_fs_list", id: "fs-bad2",
    root: "/srv/../workspaces", path: "/srv/../workspaces" });
  assert.equal((await fsReply(h, "fs-bad2")).success, false);
  assert.equal(h.childWrites.length, before);
});

test("5B can save a remote UTF-8 file after verifying the read revision", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const crypto = require("node:crypto");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-edit-ok-"));
  try {
    const file = path.join(root, "notes.txt");
    fs.writeFileSync(file, "first line\n", { mode: 0o640 });
    const h = harness(root);
    const initialCalls = h.childWrites.length;
    rpcInput(h, { type: "herdr_fs_read", id: "edit-r", root, path: file });
    const opened = await fsReply(h, "edit-r");
    assert.equal(opened.success, true);
    assert.match(opened.data.revision, /^[a-f0-9]{64}$/);

    rpcInput(h, {
      type: "herdr_fs_write", id: "edit-w", root, path: file,
      content: "Updated from Windows 🧪\n",
      expectedRevision: opened.data.revision,
    });
    const saved = await fsReply(h, "edit-w");
    assert.equal(saved.success, true);
    assert.equal(fs.readFileSync(file, "utf8"), "Updated from Windows 🧪\n");
    assert.equal(saved.data.revision,
      crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex"));
    assert.equal(fs.statSync(file).mode & 0o777, 0o640);
    assert.equal(h.childWrites.length, initialCalls,
      "private writes must never go to Pi RPC or the chat transcript");
    assert.equal(fs.readdirSync(root).filter(x => x.startsWith(".herdr-save-")).length, 0);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5B rejects stale revisions and never overwrites concurrent edits", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-edit-conflict-"));
  try {
    const file = path.join(root, "notes.txt");
    fs.writeFileSync(file, "original\n");
    const h = harness(root);
    rpcInput(h, { type: "herdr_fs_read", id: "conflict-r", root, path: file });
    const opened = await fsReply(h, "conflict-r");
    assert.equal(opened.success, true);
    fs.writeFileSync(file, "external editor changed this\n");
    rpcInput(h, { type: "herdr_fs_write", id: "conflict-w", root, path: file,
      content: "STALE SAVE", expectedRevision: opened.data.revision });
    const saved = await fsReply(h, "conflict-w");
    assert.equal(saved.success, false);
    assert.match(saved.error, /Conflict/);
    assert.equal(fs.readFileSync(file, "utf8"), "external editor changed this\n");
    assert.equal(fs.readdirSync(root).filter(x => x.startsWith(".herdr-save-")).length, 0);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5B denies writes outside root, through symlinks, and without a revision", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-edit-deny-"));
  try {
    const file = path.join(root, "text.txt");
    const escaped = path.join(path.dirname(root), "outside-edit-"+
      path.basename(root) + ".txt");
    fs.writeFileSync(file, "original");
    fs.writeFileSync(escaped, "outside");
    fs.symlinkSync(escaped, path.join(root, "alias.txt"));
    const h = harness(root);
    const hash = require("node:crypto").createHash("sha256")
      .update("original").digest("hex");
    const requests = [
      { id: "deny-1", path: escaped, expectedRevision: hash },
      { id: "deny-2", path: path.join(root, "alias.txt"), expectedRevision: hash },
      { id: "deny-3", path: file, expectedRevision: "" },
      { id: "deny-4", path: file, expectedRevision: hash, content: "x".repeat(128 * 1024 + 1) },
    ];
    for (const request of requests) {
      rpcInput(h, { type: "herdr_fs_write", root, path: request.path,
        content: request.content ?? "bad", expectedRevision: request.expectedRevision,
        id: request.id });
      const reply = await fsReply(h, request.id);
      assert.equal(reply.success, false, request.id);
    }
    assert.equal(fs.readFileSync(file, "utf8"), "original");
    assert.equal(fs.readFileSync(escaped, "utf8"), "outside");
    assert.equal(fs.readdirSync(root).filter(x => x.startsWith(".herdr-save-")).length, 0);
    fs.unlinkSync(escaped);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5B requires an existing regular file and rejects binary replacement", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-edit-types-"));
  try {
    const file = path.join(root, "binary.dat");
    fs.writeFileSync(file, Buffer.from([0, 1, 2]));
    const h = harness(root);
    const hash = require("node:crypto").createHash("sha256")
      .update(fs.readFileSync(file)).digest("hex");
    rpcInput(h, { type: "herdr_fs_write", id: "types-1",
      root, path: file, content: "text", expectedRevision: hash });
    assert.equal((await fsReply(h, "types-1")).success, false);
    rpcInput(h, { type: "herdr_fs_write", id: "types-2",
      root, path: path.join(root, "missing.txt"),
      content: "text", expectedRevision: hash });
    assert.equal((await fsReply(h, "types-2")).success, false);
    assert.deepEqual(fs.readFileSync(file), Buffer.from([0, 1, 2]));
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5B refuses editing when the remote agent root is the filesystem root", {
  skip: process.platform === "win32",
}, async () => {
  const h = harness("/");
  rpcInput(h, {
    type: "herdr_fs_write", id: "root-denied", root: "/",
    path: "/tmp/nonexistent-herdr-write.txt",
    content: "forbidden", expectedRevision: "0".repeat(64),
  });
  const result = await fsReply(h, "root-denied");
  assert.equal(result.success, false);
  assert.match(result.error, /project directory, not/);
});

function gitInit(root) {
  const cp = require("node:child_process");
  const git = (...args) => {
    const result = cp.spawnSync("git", [
      "-C", root, ...args,
    ], { encoding: "utf8", timeout: 10000 });
    assert.equal(result.status, 0, result.stderr);
  };
  git("init", "-q");
  fs.writeFileSync(path.join(root, "tracked.txt"), "first\n");
  git("add", "--", "tracked.txt");
  git("-c", "user.name=Herdr", "-c", "user.email=herdr@example.invalid",
    "commit", "-qm", "Initial");
  return git;
}

test("5C reports remote Git changes and branches without sending Pi commands", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-status-"));
  try {
    const git = gitInit(root);
    git("branch", "feature/test");
    fs.writeFileSync(path.join(root, "tracked.txt"), "changed\n");
    git("add", "--", "tracked.txt");
    fs.writeFileSync(path.join(root, "tracked.txt"), "changed again\n");
    fs.writeFileSync(path.join(root, "new.txt"), "untracked\n");
    const h = harness(root);
    const before = h.childWrites.length;
    rpcInput(h, { type: "herdr_git_snapshot", id: "git-1", root });
    const result = await fsReply(h, "git-1");
    assert.equal(result.success, true, result.error);
    assert.equal(result.data.toplevel, root);
    assert.equal(result.data.readOnly, true);
    assert.ok(result.data.files.some(x => x.path === "tracked.txt" &&
      x.index === "M" && x.worktree === "M"));
    assert.ok(result.data.files.some(x => x.path === "new.txt" &&
      x.index === "?" && x.worktree === "?"));
    assert.ok(result.data.branches.some(x => x.name === "feature/test"));
    assert.ok(result.data.branches.some(x => x.current));
    assert.equal(h.childWrites.length, before);
    assert.equal(h.calls.length, 1, "no additional agents or terminals");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5C shows tracked and untracked diffs read-only", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-diff-"));
  try {
    const git = gitInit(root);
    fs.writeFileSync(path.join(root, "tracked.txt"), "changed\n");
    git("add", "--", "tracked.txt");
    fs.writeFileSync(path.join(root, "tracked.txt"), "latest\n");
    fs.writeFileSync(path.join(root, "new.txt"), "untracked\n");
    const h = harness(root);
    for (const [id, file, expected] of [
      ["diff-tracked", "tracked.txt", "+latest"],
      ["diff-untracked", "new.txt", "+untracked"],
    ]) {
      rpcInput(h, {
        type: "herdr_git_diff", id, root, path: path.join(root, file),
      });
      const reply = await fsReply(h, id);
      assert.equal(reply.success, true, reply.error);
      assert.match(reply.data.patch, /diff --git/);
      assert.ok(reply.data.patch.includes(expected), reply.data.patch);
    }
    assert.equal(fs.readFileSync(path.join(root, "tracked.txt"), "utf8"), "latest\n");
    assert.equal(fs.readFileSync(path.join(root, "new.txt"), "utf8"), "untracked\n");
    assert.equal(h.childWrites.length, 1, "Git must not leak into Pi RPC");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5C rejects external, traversed, symlinked and unchanged Git diff paths", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-deny-"));
  const outside = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-outside-"));
  try {
    gitInit(root);
    fs.writeFileSync(path.join(outside, "secret.txt"), "secret!");
    fs.symlinkSync(path.join(outside, "secret.txt"), path.join(root, "alias"));
    const h = harness(root);
    const paths = [
      path.join(outside, "secret.txt"),
      root + "/../" + path.basename(outside) + "/secret.txt",
      path.join(root, "alias"),
      path.join(root, "tracked.txt"), // unchanged
      "C:\\Windows\\file.txt",
    ];
    for (let i = 0; i < paths.length; i++) {
      const id = "git-deny-" + i;
      rpcInput(h, { type: "herdr_git_diff", id, root, path: paths[i] });
      const reply = await fsReply(h, id);
      assert.equal(reply.success, false, id);
    }
    assert.equal(fs.readFileSync(path.join(outside, "secret.txt"), "utf8"), "secret!");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
    fs.rmSync(outside, { recursive: true, force: true });
  }
});

test("5C refuses a Git repository above the remote agent launch directory", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-parent-"));
  try {
    gitInit(root);
    fs.mkdirSync(path.join(root, "child"));
    const child = path.join(root, "child");
    const h = harness(child);
    rpcInput(h, { type: "herdr_git_snapshot", id: "git-outside", root: child });
    const reply = await fsReply(h, "git-outside");
    assert.equal(reply.success, false);
    assert.match(reply.error, /outside this agent/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5C never forwards Git requests with invalid roots to the Pi agent", async () => {
  const h = harness();
  const before = h.childWrites.length;
  rpcInput(h, {
    type: "herdr_git_snapshot", id: "git-bad-root",
    root: "C:\\Users\\javij",
  });
  const reply = await fsReply(h, "git-bad-root");
  assert.equal(reply.success, false);
  assert.equal(h.childWrites.length, before);
});

test("5C caps very large Git status without maxBuffer errors and diffs late paths", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-many-"));
  try {
    gitInit(root);
    // Shared long nested path makes 240 entries exceed the old 384-KiB
    // execFile maxBuffer limit while keeping each filename valid on ext4.
    let folder = root;
    for (let i = 0; i < 8; i++) {
      folder = path.join(folder, "nested-" + i + "-" + "x".repeat(215));
      fs.mkdirSync(folder);
    }
    for (let i = 0; i < 240; i++) {
      fs.writeFileSync(path.join(folder, "untracked-" + i + ".txt"), "hi\n");
    }
    const h = harness(root);
    const before = h.childWrites.length;
    rpcInput(h, { type: "herdr_git_snapshot", id: "git-many", root });
    const snapshot = await fsReply(h, "git-many");
    assert.equal(snapshot.success, true, snapshot.error);
    assert.equal(snapshot.data.files.length, 200);
    assert.equal(snapshot.data.filesTruncated, true);
    assert.equal(snapshot.data.readOnly, true);

    // The file's own status is small despite the huge full-tree status.
    const last = path.join(folder, "untracked-239.txt");
    rpcInput(h, { type: "herdr_git_diff", id: "git-late", root, path: last });
    const diff = await fsReply(h, "git-late");
    assert.equal(diff.success, true, diff.error);
    assert.match(diff.data.patch, /\+hi/);
    assert.equal(h.childWrites.length, before);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
