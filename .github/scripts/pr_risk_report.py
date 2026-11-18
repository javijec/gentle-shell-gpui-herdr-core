#!/usr/bin/env python3
"""Flags what a reviewer must read closely in a pull request.

Compares HEAD with the merge base of `--base` and reports:

- files in sensitive places (CI, build scripts, dependencies, packaging, the
  updater, process spawning), which can run code on a CI runner, on a
  contributor's machine at build time, or on users' machines;
- binary files, which can't be reviewed as text;
- added lines with patterns worth a second look (processes, network, `unsafe`,
  secrets, encoded payloads, invisible characters, very long lines);
- bidirectional Unicode controls in added lines ("Trojan Source"), which make
  code read differently than it compiles. This one fails the check.

It's a reading aid, not a malware scanner: a clean report doesn't mean a PR is
safe, and a flagged one isn't necessarily malicious. Output goes to stdout,
GitHub annotations (on Actions) and `$GITHUB_STEP_SUMMARY` when set.

Usage: pr_risk_report.py --base origin/main
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass

# (glob-ish prefix or regex, why it matters)
SENSITIVE_PATHS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"^\.github/"), "CI, ownership or repo automation"),
    (re.compile(r"(^|/)build\.rs$"), "runs at compile time on every machine that builds"),
    (re.compile(r"(^|/)Cargo\.(toml|lock)$"), "dependencies (supply chain)"),
    (re.compile(r"^(\.cargo/|rust-toolchain)"), "build configuration"),
    (re.compile(r"^deny\.toml$"), "dependency policy"),
    (re.compile(r"^packaging/"), "release packaging (what users install)"),
    (re.compile(r"^crates/app/src/features/app_update/"), "self-update (download, verify, replace the app)"),
    (re.compile(r"^crates/app/src/features/setup/"), "downloads and installs tools"),
    (re.compile(r"^crates/pi-rpc/"), "spawns the agent process and resolves its launcher"),
    (re.compile(r"\.(sh|ps1|bat|cmd|py|swift|iss)$"), "script"),
]

# Added-line patterns: (regex, label). Kept specific to avoid drowning reviewers.
LINE_PATTERNS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"\b(Command::new|std::process|process::Command|CreateProcess|ShellExecute)\b"), "starts a process"),
    (re.compile(r"\b(ureq::|reqwest::|TcpStream|UdpSocket|hyper::)"), "network access"),
    (re.compile(r"https?://(?!github\.com/Gentleman-Programming/|api\.github\.com/repos/Gentleman-Programming/)[^\s\"')>]+"), "new URL"),
    (re.compile(r"\bunsafe\b"), "unsafe code"),
    (re.compile(r"\b(include_bytes!|include_str!)"), "embeds a file in the binary"),
    (re.compile(r"\b(base64|from_hex|hex::decode|BASE64|STANDARD\.decode)\b"), "decodes encoded data"),
    (re.compile(r"(?i)\b(api[_-]?key|secrets?|password|passwd|private[_-]?key)\b"), "touches secrets"),
    (re.compile(r"(\.ssh|\.aws|\.gnupg|\.kube|id_rsa|id_ed25519|Keychain|login\.keychain|\.npmrc|\.netrc|GITHUB_TOKEN|ACTIONS_RUNTIME_TOKEN)"), "sensitive file or credential"),
    (re.compile(r"(?i)\b(curl|wget|invoke-webrequest|iwr)\b.*(\||;|&&)\s*(ba|z)?sh\b|\b(invoke-expression|iex)\b"), "downloads and runs code"),
    (re.compile(r"\b(remove_dir_all|set_permissions|chmod|fs::remove_file)\b"), "deletes files or changes permissions"),
    (re.compile(r"\b(env::var|std::env::vars|set_var)\b"), "environment variables"),
    (re.compile(r"\b(pull_request_target|workflow_run|secrets\.|id-token:\s*write|contents:\s*write|GITHUB_ENV|GITHUB_PATH)\b"), "privileged CI construct"),
]

# Trojan Source (CVE-2021-42574): bidirectional controls reorder how code reads.
BIDI = re.compile("[\u202a-\u202e\u2066-\u2069\u200e\u200f]")
# Invisible characters that can hide differences between look-alike names.
# U+200D (zero-width joiner) is left out: emoji sequences use it.
INVISIBLE = re.compile("[\u200b\u200c\u2060\ufeff\u00ad]")
LONG_LINE = 400


@dataclass
class Hit:
    path: str
    line: int
    label: str
    text: str


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def changed_files(base: str) -> list[tuple[str, str, bool]]:
    """(status, path, is_binary) for every file changed since the merge base."""
    status = {}
    for row in git("diff", "--name-status", "--no-renames", f"{base}...HEAD").splitlines():
        code, path = row.split("\t", 1)
        status[path] = code
    binary = set()
    for row in git("diff", "--numstat", "--no-renames", f"{base}...HEAD").splitlines():
        added, removed, path = row.split("\t", 2)
        if added == "-" and removed == "-":
            binary.add(path)
    return [(s, p, p in binary) for p, s in status.items()]


def added_lines(base: str):
    """Yields (path, line number in the new file, text) for every added line."""
    diff = git("diff", "--unified=0", "--no-color", "--no-ext-diff", f"{base}...HEAD")
    path = None
    line = 0
    hunk = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")
    for raw in diff.splitlines():
        if raw.startswith("+++ "):
            path = raw[6:] if raw.startswith("+++ b/") else None
        elif raw.startswith("@@"):
            m = hunk.match(raw)
            line = int(m.group(1)) if m else 0
        elif raw.startswith("+") and path:
            yield path, line, raw[1:]
            line += 1


def escape(text: str) -> str:
    return text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True, help="base ref, e.g. origin/main")
    args = parser.parse_args()

    files = changed_files(args.base)
    sensitive = []
    binaries = []
    for status, path, is_binary in files:
        reasons = [why for rx, why in SENSITIVE_PATHS if rx.search(path)]
        if reasons:
            sensitive.append((status, path, ", ".join(dict.fromkeys(reasons))))
        if is_binary:
            binaries.append((status, path))

    hits: list[Hit] = []
    hidden: list[Hit] = []
    for path, line, text in added_lines(args.base):
        if BIDI.search(text):
            shown = BIDI.sub(lambda m: f"<U+{ord(m.group()):04X}>", text)
            hidden.append(Hit(path, line, "bidirectional Unicode control", shown.strip()))
        if INVISIBLE.search(text):
            shown = INVISIBLE.sub(lambda m: f"<U+{ord(m.group()):04X}>", text)
            hits.append(Hit(path, line, "invisible character", shown.strip()[:160]))
        # Generated files: their changes are checked by cargo-deny, not by eye.
        if path.endswith((".lock", ".svg", ".json")):
            continue
        if len(text) > LONG_LINE:
            hits.append(Hit(path, line, f"line longer than {LONG_LINE} characters", text[:120] + "…"))
        for rx, label in LINE_PATTERNS:
            if rx.search(text):
                hits.append(Hit(path, line, label, text.strip()[:160]))

    on_actions = os.environ.get("GITHUB_ACTIONS") == "true"
    out = []
    out.append("## PR risk report\n")
    out.append(
        "A reading aid for reviewers: what to read line by line. "
        "Not a malware scanner; a clean report does not make a PR safe.\n"
    )

    if hidden:
        out.append("### ❌ Bidirectional Unicode controls\n")
        out.append("Code can read differently than it compiles (Trojan Source). Remove these characters.\n")
        out.append("| File | Line | Text |\n|---|---|---|")
        for h in hidden:
            out.append(f"| `{h.path}` | {h.line} | `{h.text[:120]}` |")
            if on_actions:
                print(f"::error file={h.path},line={h.line},title=Bidirectional Unicode::{escape(h.text[:200])}")
        out.append("")

    out.append(f"### Sensitive files ({len(sensitive)})\n")
    if sensitive:
        out.append("| | File | Why it matters |\n|---|---|---|")
        for status, path, why in sensitive:
            out.append(f"| {status} | `{path}` | {why} |")
            if on_actions:
                print(f"::warning file={path},title=Sensitive file::{escape(why)}")
    else:
        out.append("None.")
    out.append("")

    out.append(f"### Binary files ({len(binaries)})\n")
    if binaries:
        out.append("Not reviewable as text: confirm where each one comes from.\n")
        out.append("| | File |\n|---|---|")
        for status, path in binaries:
            out.append(f"| {status} | `{path}` |")
            if on_actions:
                print(f"::warning file={path},title=Binary file::Binary content can't be reviewed in the diff.")
    else:
        out.append("None.")
    out.append("")

    out.append(f"### Added lines worth a second look ({len(hits)})\n")
    if hits:
        out.append("| File | Line | What | Text |\n|---|---|---|---|")
        for h in hits:
            cell = h.text.replace("|", "\\|").replace("`", "'")
            out.append(f"| `{h.path}` | {h.line} | {h.label} | `{cell}` |")
            if on_actions:
                print(f"::notice file={h.path},line={h.line},title={h.label}::{escape(h.text)}")
    else:
        out.append("None.")
    out.append("")

    report = "\n".join(out)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(report + "\n")

    return 1 if hidden else 0


if __name__ == "__main__":
    sys.exit(main())
