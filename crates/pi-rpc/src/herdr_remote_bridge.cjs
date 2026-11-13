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

// The PTY is line-buffered: GPUI submits each encoded input frame with CR.
// Accept CR, LF and CRLF, echo/control-character noise and stream splits.
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
    agent.stdin.write(payload);
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
