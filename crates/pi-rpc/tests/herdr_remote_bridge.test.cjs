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
    stdout: { write: (value) => stdout.push(value) },
    stderr: { write: () => {} },
    stdin: input,
    on: () => {},
    exit: (code) => { throw Error("unexpected exit: " + code); },
  };
  const requireMock = (name) => {
    if (name === "node:child_process") {
      return { spawn: (...args) => { calls.push(args); return child; } };
    }
    // The remote Files extension only requires Node built-ins and
    // performs no I/O until it receives a specific read-only request.
    assert.ok(["node:fs/promises", "node:path", "node:util"].includes(name));
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
  for (let attempt = 0; attempt < 100; attempt++) {
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
