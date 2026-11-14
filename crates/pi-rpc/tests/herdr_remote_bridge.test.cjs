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
  git("config", "user.name", "Herdr");
  git("config", "user.email", "herdr@example.invalid");
  fs.writeFileSync(path.join(root, "tracked.txt"), "first\n");
  git("add", "--", "tracked.txt");
  git("-c", "user.name=Herdr", "-c", "user.email=herdr@example.invalid",
    "commit", "-qm", "Initial");
  return git;
}

test("5F creates a file or folder only inside the selected Herdr root", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-create-"));
  const outside = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-create-outside-"));
  try {
    const h = harness(root);
    const create = async (id, parent, name, kind) => {
      rpcInput(h, {
        type: "herdr_fs_create", id, root, path: parent, name, kind,
      });
      return await fsReply(h, id);
    };
    const directory = await create("create-dir", root, "src", "directory");
    assert.equal(directory.success, true, directory.error);
    assert.equal(fs.statSync(path.join(root, "src")).isDirectory(), true);
    const file = await create("create-file", path.join(root, "src"), "main.rs", "file");
    assert.equal(file.success, true, file.error);
    assert.equal(fs.readFileSync(path.join(root, "src/main.rs"), "utf8"), "");
    assert.equal((await create("create-duplicate",
      path.join(root, "src"), "main.rs", "file")).success, false);
    assert.equal((await create("create-traverse",
      root, "../outside", "directory")).success, false);
    assert.equal((await create("create-git", root, ".git", "directory")).success, false);
    fs.mkdirSync(path.join(root, ".git"));
    fs.writeFileSync(path.join(root, ".git", "config"), "[core]\\n");
    assert.equal((await create("create-in-git", path.join(root, ".git"),
      "unsafe", "file")).success, false);
    rpcInput(h, {
      type: "herdr_fs_write", id: "write-git-config", root,
      path: path.join(root, ".git", "config"), content: "tampered",
      expectedRevision: "a".repeat(64),
    });
    assert.equal((await fsReply(h, "write-git-config")).success, false);
    assert.equal(fs.existsSync(path.join(root, ".git", "unsafe")), false);

    fs.symlinkSync(outside, path.join(root, "external"), "dir");
    assert.equal((await create("create-symlink",
      path.join(root, "external"), "escape", "file")).success, false);
    assert.equal(fs.existsSync(path.join(outside, "escape")), false);
    assert.equal(h.childWrites.length, 1, "no additional Pi agent RPC operations");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
    fs.rmSync(outside, { recursive: true, force: true });
  }
});

test("5F guarded remote file entry actions never overwrite or escape", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-entry-"));
  const outside = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-entry-outside-"));
  try {
    fs.mkdirSync(path.join(root, "src"));
    fs.mkdirSync(path.join(root, "dest"));
    fs.writeFileSync(path.join(root, "src", "main.rs"), "original\n");
    fs.writeFileSync(path.join(root, "dest", "main.rs"), "existing\n");
    fs.writeFileSync(path.join(outside, "secret"), "private");
    fs.symlinkSync(outside, path.join(root, "external"), "dir");
    const h = harness(root);
    const act = async (id, operation, source, props = {}) => {
      rpcInput(h, {
        type: "herdr_fs_entry", id, root, path: source,
        operation, ...props,
      });
      return fsReply(h, id);
    };
    let result = await act("entry-rename", "rename",
      path.join(root, "src", "main.rs"), { name: "renamed.rs" });
    assert.equal(result.success, true, result.error);
    assert.equal(fs.existsSync(path.join(root, "src", "main.rs")), false);
    assert.equal(fs.readFileSync(path.join(root, "src", "renamed.rs"), "utf8"), "original\n");
    result = await act("entry-duplicate", "duplicate",
      path.join(root, "src", "renamed.rs"));
    assert.equal(result.success, true, result.error);
    assert.equal(fs.readFileSync(path.join(root, "src", "renamed copy.rs"), "utf8"),
      "original\n");
    result = await act("entry-copy", "copy",
      path.join(root, "src", "renamed.rs"), {
        destinationDir: path.join(root, "dest"),
      });
    assert.equal(result.success, true, result.error);
    assert.equal(fs.readFileSync(path.join(root, "dest", "renamed.rs"), "utf8"),
      "original\n");
    const conflict = await act("entry-conflict", "rename",
      path.join(root, "src", "renamed.rs"), { name: "renamed copy.rs" });
    assert.equal(conflict.success, false, "rename must not overwrite");
    result = await act("entry-move", "move",
      path.join(root, "src", "renamed.rs"), {
        destinationDir: path.join(root, "dest"),
      });
    assert.equal(result.success, false, "move must refuse a duplicate name");
    assert.equal(fs.readFileSync(path.join(root, "dest", "renamed.rs"), "utf8"),
      "original\n");
    result = await act("entry-folder", "rename",
      path.join(root, "src"), { name: "source" });
    assert.equal(result.success, true, result.error);
    assert.equal(fs.existsSync(path.join(root, "source", "renamed copy.rs")), true);
    const escape = await act("entry-escape", "copy",
      path.join(root, "source", "renamed copy.rs"), { destinationDir: outside });
    assert.equal(escape.success, false);
    const symlink = await act("entry-symlink", "move",
      path.join(root, "source", "renamed copy.rs"), {
        destinationDir: path.join(root, "external"),
      });
    assert.equal(symlink.success, false);
    assert.equal(fs.existsSync(path.join(outside, "renamed copy.rs")), false);
    assert.equal((await act("entry-unsafe", "rename",
      path.join(root, "source", "renamed copy.rs"), { name: "../outside" })).success,
      false);
    assert.equal((await act("entry-root", "rename", root, { name: "evil" })).success,
      false);
    assert.equal(h.childWrites.length, 1, "only initial Pi startup RPC was forwarded");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
    fs.rmSync(outside, { recursive: true, force: true });
  }
});

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

test("5E.6 includes bounded tag and stash metadata without Pi RPC writes", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-extra-refs-"));
  try {
    const git = gitInit(root);
    git("tag", "v1.0");
    fs.writeFileSync(path.join(root, "tracked.txt"), "modified\n");
    git("stash", "push", "-qm", "wip: local experiment");
    const h = harness(root);
    const before = h.childWrites.length;
    rpcInput(h, { type: "herdr_git_snapshot", id: "refs-extra", root });
    const reply = await fsReply(h, "refs-extra");
    assert.equal(reply.success, true, reply.error);
    assert.ok(reply.data.tags.some((tag) => tag.name === "v1.0"));
    assert.ok(reply.data.stashes.some((stash) =>
      stash.name === "stash@{0}" &&
      stash.message.includes("wip: local experiment")));
    assert.equal(reply.data.tagsTruncated, false);
    assert.equal(reply.data.stashesTruncated, false);
    assert.equal(h.childWrites.length, before);
    assert.equal(h.calls.length, 1);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5E.6 caps tag and stash listings without maxBuffer errors", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-extra-cap-"));
  try {
    const git = gitInit(root);
    for (let i = 0; i < 125; i++) {
      git("tag", "test-tag-" + i.toString().padStart(3, "0"));
    }
    const h = harness(root);
    rpcInput(h, { type: "herdr_git_snapshot", id: "refs-cap", root });
    const reply = await fsReply(h, "refs-cap");
    assert.equal(reply.success, true, reply.error);
    assert.equal(reply.data.tags.length, 120);
    assert.equal(reply.data.tagsTruncated, true);
    assert.deepEqual(reply.data.stashes, []);
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


// 5D mutations use the exact HEAD/branch from the preceding Git snapshot.
async function gitSnapshot(h, root, id) {
  rpcInput(h, { type: "herdr_git_snapshot", id, root });
  const result = await fsReply(h, id);
  assert.equal(result.success, true, result.error);
  return result.data;
}

async function gitWrite(h, root, snapshot, type, id, props = {}) {
  rpcInput(h, {
    type, id, root, toplevel: snapshot.toplevel,
    expectedHead: snapshot.headOid, expectedBranch: snapshot.currentBranch,
    expectedIndexToken: snapshot.indexToken,
    ...props,
  });
  return fsReply(h, id);
}

test("5F bulk Git stage and unstage are bounded and revalidate status", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-bulk-git-"));
  try {
    gitInit(root);
    fs.writeFileSync(path.join(root, "tracked.txt"), "changed one\\n");
    fs.writeFileSync(path.join(root, "second.txt"), "new file\\n");
    const h = harness(root);
    let snap = await gitSnapshot(h, root, "bulk-s1");
    const original = snap.files.map(({ path, index, worktree }) => ({
      path, index, worktree,
    }));
    const success = await gitWrite(h, root, snap, "herdr_git_stage_all", "bulk-stage", {
      expectedFiles: original,
    });
    assert.equal(success.success, true, success.error);
    assert.equal((await gitWrite(h, root, snap, "herdr_git_unstage_all", "bulk-stale", {
      expectedFiles: original,
    })).success, false, "index guard must block a stale request");
    snap = await gitSnapshot(h, root, "bulk-s2");
    assert.ok(snap.files.every((f) => f.index !== " " && f.index !== "?"));
    const reviewed = snap.files.map(({ path, index, worktree }) => ({
      path, index, worktree,
    }));
    const reverted = await gitWrite(h, root, snap, "herdr_git_unstage_all",
      "bulk-unstage", { expectedFiles: reviewed });
    assert.equal(reverted.success, true, reverted.error);
    assert.equal(fs.readFileSync(path.join(root, "tracked.txt"), "utf8"), "changed one\\n");
    assert.equal(fs.readFileSync(path.join(root, "second.txt"), "utf8"), "new file\\n");
    snap = await gitSnapshot(h, root, "bulk-s3");
    const changed = snap.files.map(({ path, index, worktree }) => ({
      path, index, worktree,
    }));
    changed[0].index = "A"; // tamper with the file the user reviewed
    const refused = await gitWrite(h, root, snap, "herdr_git_stage_all",
      "bulk-tampered", { expectedFiles: changed });
    assert.equal(refused.success, false);
    assert.match(refused.error, /Git file status changed/);
    assert.equal(h.childWrites.length, 1, "all requests remain inside the private bridge");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5D stages and unstages exactly one selected file without modifying worktree", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-write-"));
  try {
    gitInit(root);
    const file = path.join(root, "tracked.txt");
    fs.writeFileSync(file, "edited in Linux\n");
    fs.writeFileSync(path.join(root, "other.txt"), "leave me untracked\n");
    const h = harness(root);
    let snapshot = await gitSnapshot(h, root, "write-s1");
    assert.match(snapshot.headOid, /^[a-f0-9]{40}$/);
    assert.match(snapshot.indexToken, /^[a-f0-9]{64}$/);
    assert.equal(snapshot.writesEnabled, true);
    const first = snapshot.files.find(x => x.path === "tracked.txt");
    assert.equal((await gitWrite(h, root, snapshot, "herdr_git_stage", "write-a", {
      path: file, expectedIndex: first.index, expectedWorktree: first.worktree,
    })).success, true);

    snapshot = await gitSnapshot(h, root, "write-s2");
    const staged = snapshot.files.find(x => x.path === "tracked.txt");
    assert.equal(staged.index, "M");
    const undone = await gitWrite(h, root, snapshot, "herdr_git_unstage", "write-u", {
      path: file, expectedIndex: staged.index, expectedWorktree: staged.worktree,
    });
    assert.equal(undone.success, true, undone.error);
    snapshot = await gitSnapshot(h, root, "write-s3");
    assert.equal(snapshot.files.find(x => x.path === "tracked.txt").index, " ");
    assert.equal(fs.readFileSync(file, "utf8"), "edited in Linux\n");
    assert.ok(snapshot.files.some(x => x.path === "other.txt"));
    assert.equal(h.childWrites.length, 1, "mutations must not execute Pi RPC or bash");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5D commits staged files only, with an explicit message and guarded HEAD", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const cp = require("node:child_process");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-commit-"));
  try {
    gitInit(root);
    const file = path.join(root, "tracked.txt");
    fs.writeFileSync(file, "staged version\n");
    const h = harness(root);
    let snap = await gitSnapshot(h, root, "commit-s1");
    const entry = snap.files.find(x => x.path === "tracked.txt");
    assert.equal((await gitWrite(h, root, snap, "herdr_git_stage", "commit-stage", {
      path: file, expectedIndex: entry.index, expectedWorktree: entry.worktree,
    })).success, true);
    fs.writeFileSync(file, "unstaged after stage\n");
    snap = await gitSnapshot(h, root, "commit-s2");
    const noMessage = await gitWrite(h, root, snap, "herdr_git_commit", "commit-empty", {
      summary: "",
    });
    assert.equal(noMessage.success, false);
    const done = await gitWrite(h, root, snap, "herdr_git_commit", "commit-real", {
      summary: "feat: remote stage", description: "Safe stage 5D test",
    });
    assert.equal(done.success, true, done.error);
    const log = cp.execFileSync("git", ["-C", root, "log", "-1", "--format=%s"]).toString();
    assert.equal(log.trim(), "feat: remote stage");
    assert.equal(cp.execFileSync("git", ["-C", root, "show", "HEAD:tracked.txt"]).toString(),
      "staged version\n");
    assert.equal(fs.readFileSync(file, "utf8"), "unstaged after stage\n");
    const stale = await gitWrite(h, root, snap, "herdr_git_commit", "commit-stale", {
      summary: "must fail",
    });
    assert.equal(stale.success, false);
    assert.match(stale.error, /HEAD or branch changed/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5D supports first-commit stage and unstage without deleting new files", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const cp = require("node:child_process");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-unborn-"));
  try {
    cp.execFileSync("git", ["-C", root, "init", "-q", "-b", "main"]);
    const file = path.join(root, "initial.txt");
    fs.writeFileSync(file, "initial\n");
    const h = harness(root);
    let snap = await gitSnapshot(h, root, "unborn-s1");
    assert.equal(snap.headOid, null);
    const item = snap.files.find(x => x.path === "initial.txt");
    assert.equal((await gitWrite(h, root, snap, "herdr_git_stage", "unborn-stage", {
      path: file, expectedIndex: item.index, expectedWorktree: item.worktree,
    })).success, true);
    snap = await gitSnapshot(h, root, "unborn-s2");
    const staged = snap.files.find(x => x.path === "initial.txt");
    const unstaged = await gitWrite(h, root, snap, "herdr_git_unstage", "unborn-unstage", {
      path: file, expectedIndex: staged.index, expectedWorktree: staged.worktree,
    });
    assert.equal(unstaged.success, true, unstaged.error);
    assert.equal(fs.readFileSync(file, "utf8"), "initial\n");
    assert.equal((await gitSnapshot(h, root, "unborn-s3"))
      .files.find(x => x.path === "initial.txt").index, "?");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5D creates local branches and refuses switching while dirty", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const cp = require("node:child_process");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-branch-"));
  try {
    gitInit(root);
    const h = harness(root);
    let snap = await gitSnapshot(h, root, "branch-s1");
    assert.equal((await gitWrite(h, root, snap, "herdr_git_create_branch", "branch-new", {
      branch: "feature/5d",
    })).success, true);
    fs.writeFileSync(path.join(root, "untracked.txt"), "dirty\n");
    let dirty = await gitSnapshot(h, root, "branch-s2");
    const refused = await gitWrite(h, root, dirty,
      "herdr_git_switch_branch", "branch-deny", { branch: "feature/5d" });
    assert.equal(refused.success, false);
    assert.match(refused.error, /clean index and worktree/);
    fs.unlinkSync(path.join(root, "untracked.txt"));
    snap = await gitSnapshot(h, root, "branch-s3");
    const switched = await gitWrite(h, root, snap,
      "herdr_git_switch_branch", "branch-switch", { branch: "feature/5d" });
    assert.equal(switched.success, true, switched.error);
    assert.equal(cp.execFileSync("git", ["-C", root, "branch", "--show-current"])
      .toString().trim(), "feature/5d");
    assert.equal(h.childWrites.length, 1);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("5D denies stale status, symlinks, unselected paths and invalid refs", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-write-deny-"));
  const outside = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-write-outside-"));
  try {
    gitInit(root);
    const file = path.join(root, "tracked.txt");
    fs.writeFileSync(file, "changed\n");
    fs.writeFileSync(path.join(outside, "secret"), "secret\n");
    fs.symlinkSync(path.join(outside, "secret"), path.join(root, "alias"));
    const h = harness(root);
    const snapshot = await gitSnapshot(h, root, "deny-s1");
    const state = snapshot.files.find(x => x.path === "tracked.txt");
    const bad = [
      { id: "deny-stage-old", path: file, expectedIndex: "M",
        expectedWorktree: state.worktree },
      { id: "deny-stage-external", path: path.join(outside, "secret"),
        expectedIndex: "?", expectedWorktree: "?" },
      { id: "deny-stage-alias", path: path.join(root, "alias"),
        expectedIndex: "?", expectedWorktree: "?" },
      { id: "deny-stage-drive", path: "C:\\Windows\\hello.txt",
        expectedIndex: "?", expectedWorktree: "?" },
    ];
    for (const item of bad) {
      const result = await gitWrite(h, root, snapshot, "herdr_git_stage", item.id, item);
      assert.equal(result.success, false, item.id);
    }
    const withoutGuards = await gitWrite(h, root, snapshot, "herdr_git_stage",
      "deny-no-head", { path: file, expectedIndex: state.index,
        expectedWorktree: state.worktree, expectedHead: undefined });
    assert.equal(withoutGuards.success, false);
    assert.equal(fs.readFileSync(path.join(outside, "secret"), "utf8"), "secret\n");
    assert.equal(h.childWrites.length, 1);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
    fs.rmSync(outside, { recursive: true, force: true });
  }
});

test("5D refuses a commit when the staged index changed since the snapshot", {
  skip: process.platform === "win32",
}, async () => {
  const os = require("node:os");
  const cp = require("node:child_process");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "herdr-git-index-"));
  try {
    gitInit(root);
    const h = harness(root);
    fs.writeFileSync(path.join(root, "tracked.txt"), "first staged\n");
    cp.execFileSync("git", ["-C", root, "add", "tracked.txt"]);
    const old = await gitSnapshot(h, root, "index-s1");
    fs.writeFileSync(path.join(root, "tracked.txt"), "second staged\n");
    cp.execFileSync("git", ["-C", root, "add", "tracked.txt"]);
    const stale = await gitWrite(h, root, old, "herdr_git_commit",
      "index-stale", { summary: "must not commit" });
    assert.equal(stale.success, false);
    assert.match(stale.error, /staged Git index changed/);
    assert.equal(cp.execFileSync("git", ["-C", root,
      "log", "-1", "--format=%s"]).toString().trim(), "Initial");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
