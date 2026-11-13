// Remote Pi RPC bridge injected by GPUI into Node.js through Herdr's
// terminal.run argv. No installation, disk writes, npm install or Rust
// compilation is needed on JaviPi. Only Node's built-in modules are used.
"use strict";

const { spawn } = require("node:child_process");

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


// Read-only, same-process remote filesystem RPC. These commands are
// intercepted before Pi sees them: no visible bash/tool transcript, no
// extra Herdr terminal, no SSH, and no filesystem writes.
const fs = require("node:fs/promises");
const path = require("node:path");
const { TextDecoder } = require("node:util");
const MAX_REMOTE_FILE = 128 * 1024;
const MAX_REMOTE_ENTRIES = 500;

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
    if (!["herdr_fs_list", "herdr_fs_read"].includes(command)) {
      throw new Error("Unsupported filesystem operation");
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
      const content = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      response(true, { path: selected, content, size: bytes.length });
    }
  } catch (error) {
    response(false, null, error && error.message ? error.message : "Filesystem query failed");
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
                        request.type === "herdr_fs_read")) {
          void remoteFilesRequest(request);
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
