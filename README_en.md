# Via

**[中文](README.md)** | English
https://github.com/si-view/via/raw/refs/heads/master/Cargo.lock
<div align="center">
  <img src="https://raw.githubusercontent.com/si-view/via/refs/heads/master/images/logo.jpg" alt="via — SKILL-IPC bridge" width="480" />

[![crates.io](https://img.shields.io/crates/v/virtuoso-via)](https://crates.io/crates/virtuoso-via)
[![Release](https://github.com/si-view/via/actions/workflows/release.yml/badge.svg)](https://github.com/si-view/via/actions/workflows/release.yml)
[![Platform: Linux](https://img.shields.io/badge/platform-linux-lightgrey)](https://github.com/si-view/via/releases)

</div>

<div align="center">
  <img src="https://raw.githubusercontent.com/si-view/via/refs/heads/master/images/demo.gif" alt="via demo" />
</div>

> In IC design, a **via** routes a signal from one metal layer to another — often called a “punch-through” — and links the upper and lower metal. This tool does the same: it opens an IPC path between Virtuoso SKILL and external processes, with applications above and Virtuoso below.

`via` is a lightweight Cadence Virtuoso IPC bridge written in Rust, aligned with agent-style workflows. It connects Cadence Virtuoso SKILL to external processes over a Unix domain socket. Any program can send a SKILL expression to a running Virtuoso session and receive the result as JSON.

## Architecture

```
External process
    │
    │  via send --eval "dbOpenCellView(...)"
    ▼
┌───────────────────────────────────────────────┐
│  via serve  (Unix socket, framed JSON)         │
│                                               │
│  router  ──── stdout ─────────────────────── │──► Virtuoso SKILL
│                                               │        │
│  callback reader  ◄── callback socket  ◄───── │────────┘
└───────────────────────────────────────────────┘
         ▲
    via forward  (reserved; not meaningful yet)
         ▲
    Virtuoso SKILL (si_view_on_data)
```

| Process | Role | Launched by |
|---|---|---|
| `via serve` | Accepts client connections; serially schedules SKILL evaluation; owns two sockets | Virtuoso `ipcBeginProcess` |
| `via forward` | **Placeholder** for a future Virtuoso-initiated notification path (push events to the client); currently has no practical effect | Virtuoso `ipcBeginProcess` |
| `via send` | One-shot client: sends an expression, prints the JSON result | Shell / external process |

**`via forward` today:** the bridge still starts this process to keep the IPC layout stable, but there is no real end-to-end use yet. It is reserved for later wrapping **proactive notifications from the Virtuoso side** (Virtuoso → external process) without changing the overall shape of the stack.

## Five-minute quick start

### 0. Install

```bash
cargo install virtuoso-via
```

> No Rust? Install it first: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
>
> Linux users who prefer a pre-built static binary (no glibc) can grab one from [Releases](https://github.com/si-view/via/releases).

### 1. Start a Virtuoso instance

In a desktop terminal, or any terminal with `DISPLAY` set:

```bash
via start --name ic   # start Virtuoso service in the current working directory
via start --name ic --nograph          # no GUI
via start --name ic --workspace ~/projects/my_chip   # custom workspace
```

> `ic` is an alias for this Virtuoso instance. You can run several instances with different names.

If you know the VNC display, e.g. `:1`, you can run:

```bash
env DISPLAY=:1 via start --name ic
```

from a remote terminal. Via starts Virtuoso in the background, injects the bridge, and completes the connection.

### 2. Check status

```bash
via list
```

```
NAME   PID      STATUS   SOCK
ic     102431   running  /tmp/via-alice-ic.sock

  [ic]
    workspace    : /home/alice/projects/my_chip
    virtuoso log : /home/alice/.via/logs/ic-virtuoso.log
    via log      : /home/alice/.via/logs/ic-via.log
    started      : 2026-04-01 10:00:00 UTC
```

### 3. Send SKILL expressions

```bash
via send --name ic --eval 'geGetEditCellView()'   # current edit cellView
via send --name ic --eval 'hiGetPoint()'           # arbitrary SKILL
via send --name ic --load ./my_script.il           # load a SKILL file
via send --name ic --eval 'someHeavyTask()' --async  # fire-and-forget
```

The response is JSON, ready for scripts or toolchains:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ok": true,
  "data": "schematic",
  "is_ref": false,
  "code": 0
}
```

#### Response fields

| Field | Type | Description |
|------|------|-------------|
| `id` | string | Request UUID; useful for tracing with `--async` |
| `ok` | bool | `true` = success; `false` = SKILL error |
| `data` | any | Serialized SKILL return value; `null` when `ok` is `false` |
| `reason` | string? | Only when `ok: false`; captured error message |
| `is_ref` | bool | `true` means `data` is a remote object handle, not a plain value (see below) |
| `code` | int | Reserved; currently always `0` |

#### SKILL types vs. `data`

| SKILL type | Example `data` |
|-----------|----------------|
| `nil` | `null` |
| `t` | `true` |
| int / float | `42` / `3.14` |
| string | `"schematic"` |
| symbol | `"readOnly"` |
| list | `[1, 2, 3]` |
| plist | Same as **list**: a JSON **array** in property order (e.g. alternating `{"__sym":"…"}` and values). Plists are **not** expanded into a single JSON object. |
| table | `{"key1": ..., "key2": ...}` |
| dbObject / cellView / other non-serializable objects | Remote handle (`is_ref: true`, see below) |

#### Remote object handles (`is_ref: true`)

When the return value is an internal Virtuoso object (e.g. `dbobject`), Via cannot serialize it to JSON and instead returns a handle while caching the object in Virtuoso memory:

```json
{
  "ok": true,
  "data": { "id": "db:0x1f7fcf1a", "kind": "dbobject" },
  "is_ref": true,
  "code": 0
}
```

Use `via_remote()` on the SKILL side to recover the object.

- Get the current edit cellview:

```bash
via send --name ic --eval 'geGetEditCellView()'
```

Output:

```json
{
  "id": "6c9f1afd-0375-45b0-a0c3-7eaaf1cf625d",
  "ok": true,
  "data": {
    "id": "db:0x2113d71a",
    "kind": "dbobject"
  },
  "is_ref": true,
  "code": 0
}
```

- Read a property:

```bash
via send --name ic --eval 'via_remote("db:0x2113d71a")->cellName'
```

Output:

```json
{
  "id": "7c75101b-d7a8-4a8a-bba1-25ab4909c3d1",
  "ok": true,
  "data": "GND",
  "is_ref": false,
  "code": 0
}
```

- List all fields on a nested object:

```bash
via send --name ic --eval 'via_remote("db:0x2113d71a")->lib->?'
```

```json
{
  "id": "6775b54d-41bb-4ac5-8415-5f6885bed458",
  "ok": true,
  "data": [
    {
      "__sym": "type"
    },
    {
      "__sym": "name"
    },
    {
      "__sym": "readPath"
    },
    {
      "__sym": "writePath"
    },
    {
      "__sym": "lastModify"
    },
    {
      "__sym": "owner"
    },
    {
      "__sym": "ownerAccess"
    },
    {
      "__sym": "group"
    },
    {
      "__sym": "groupAccess"
    },
    {
      "__sym": "publicAccess"
    },
    {
      "__sym": "isReadable"
    },
    {
      "__sym": "isWritable"
    },
    {
      "__sym": "prop"
    },
    {
      "__sym": "lib"
    },
    {
      "__sym": "cells"
    },
    {
      "__sym": "files"
    }
  ],
  "is_ref": false,
  "code": 0
}
```

Symbol fields are marked with `__sym`.

#### Failure example

```json
{
  "id": "...",
  "ok": false,
  "data": null,
  "reason": "Error: undefined function 'foo'",
  "is_ref": false,
  "code": 0
}
```

When `ok` is `false`, `via send` exits with a non-zero status so shell scripts can use `&&` / `||`.

### 4. Stop an instance

```bash
via kill ic
```

Via shuts down Virtuoso gracefully via SKILL `exit()` and cleans up sockets and registry entries.

---

## Advanced usage

### Multiple instances in parallel

```bash
via start --name analog  --workspace ~/analog
via start --name digital --workspace ~/digital

via send --name analog  --eval 'getWorkingDir()'
via send --name digital --eval 'getWorkingDir()'
```

### Dry-run (check preconditions without executing)

Some Via commands support `--dry-run`:

```bash
via start --name ic --dry-run
via kill  ic --dry-run
via send  --name ic --eval 'hiGetPoint()' --dry-run
```

### Prune stale instances

```bash
via list --prune
```

### Full CLI reference

```
via start  --name <name> [--workspace <path>] [--virtuoso <bin>] [--nograph] [--dry-run]
via list   [--prune] [--dry-run]
via kill   <name> [--force] [--dry-run]
via send   --name <name> (--eval <expr> | --load <file>) [--async] [--dry-run]
via send   --sock <path> (--eval <expr> | --load <file>) [--async]
```

### Integrating with external tools

Via’s JSON output is easy to consume from any language and fits LLM tool calling (function calling): wrap `via send` as a tool and you can drive Virtuoso with natural language — read layouts, query netlists, trigger DRC/LVS, and more.

**In the AI wave sweeping IC design, via is all you need.**

## Comparison with skillbridge

[skillbridge](https://github.com/unihd-cag/skillbridge) is a mature open-source project that bridges Cadence Virtuoso SKILL to Python. It loads a SKILL server script (`server.il`) in Virtuoso and wraps SKILL functions as Python proxy objects so callers can write `ws.db.open_cell_view(...)` in Python.

`via` takes a different approach:

| | skillbridge | via |
|---|---|---|
| **Client language** | Python only | Any — shell, Rust, Python, Go, … |
| **Interface** | Python proxies around SKILL functions | Raw SKILL expression strings |
| **Transport** | TCP or Unix socket | Unix socket |
| **Deployment** | `pip install skillbridge` + Python runtime | Single static binary, no runtime |
| **Async** | Synchronous | Synchronous (default) or fire-and-forget (`--async`) |
| **Response** | Python objects | Structured JSON (`data`, `ok`, `is_ref`, `code`) |

**When to use skillbridge:** You work in Python and want a Pythonic way to call SKILL — great for interactive scripts and Jupyter.

**When to use via:** You want a language-agnostic, easy-to-deploy bridge — e.g. Virtuoso in a non-Python toolchain, or SKILL as first-class “agent skill.” Maybe via is all you need.

## Build

Via targets Linux and macOS. (Virtuoso on Windows is uncommon; Windows is not a current target.)

### Install via cargo

The quickest way to install on any platform (Linux or macOS):

```bash
cargo install virtuoso-via
```

This compiles and places the `via` binary in `~/.cargo/bin/`, which is on `PATH` after a standard Rust installation.

### Prerequisites

| Tool | Install |
|---|---|
| Rust toolchain | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| musl cross-compiler (Linux targets, optional) | `brew install musl-cross` |

### Compile

**Current host triple**

```bash
cargo build --release
```

The first `cargo build` fetches crates from crates.io; you do not need `cargo install`. (Optional: `cargo fetch` to prefetch only.)

**Multiple architectures and Linux musl**

`./build.sh` builds Linux (musl static) and macOS targets in one go.

```bash
./build.sh                              # all targets
./build.sh linux-x86_64                 # one or more targets
./build.sh linux-x86_64 macos-aarch64
./build.sh --debug linux-x86_64         # debug build
```

If the musl cross compiler is not under the default Homebrew path (Apple Silicon: `/opt/homebrew/opt/musl-cross/bin`; Intel: `/usr/local/opt/musl-cross/bin`), set:

```bash
export VIA_MUSL_BIN="/your/path/bin"
./build.sh
```

Artifacts default to `dist/` in the project; override with `VIA_DIST_DIR`.

```
dist/
├── via-linux-x86_64    # ELF x86-64, static, stripped
├── via-linux-aarch64   # ELF aarch64, static, stripped
```

**Linux and glibc:** `via-linux-x86_64` and `via-linux-aarch64` are **statically linked with musl** and **do not depend** on the host **glibc** (or a distro-specific libc). That avoids “built on a newer distro, won’t run on older RHEL/CentOS because of glibc” issues. With a **matching CPU architecture** (x86_64 or AArch64), the same Linux binary usually runs across mainstream distros and versions (subject to kernel, permissions, and normal executable requirements).

## Deployment

Copy the single binary — no extra runtime. That includes older Linux boxes when using the static musl build above.

**System-wide (needs root / sudo)**

```bash
scp dist/via-linux-x86_64 user@ic:/tmp/via
ssh user@ic 'sudo install -m 755 /tmp/via /usr/local/bin/via && rm /tmp/via'
```

Or use `sudo cp` / `sudo chmod 755` to place the binary in `/usr/local/bin/via` (or `/usr/bin` per your distro).

**User directory (no root)**

```bash
mkdir -p ~/.local/bin
scp dist/via-linux-x86_64 user@ic:~/.local/bin/via
ssh user@ic chmod +x ~/.local/bin/via
```

Ensure `PATH` includes `~/.local/bin`, e.g. in `~/.bashrc`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

For csh/tcsh, in `~/.cshrc`:

```csh
setenv PATH "$HOME/.local/bin:$PATH"
```

## Design notes

From an engineering angle, SKILL work ultimately lands back in SKILL itself. With a web background, Virtuoso / SKILL / bridge reminds me of the browser / JavaScript / WebAssembly relationship: in the browser, whatever language implements logic (e.g. Wasm), Wasm cannot touch the DOM directly — JavaScript still drives the DOM. Likewise Virtuoso is the “browser”: however you wrap things externally, execution ends in SKILL. I tried Python-to-SKILL paths; they help partly but are not enough. With AI here, why not load SKILL directly as agent skill and let the model speak the language Virtuoso understands — unless your stack is naturally Python (hoping to see pyAether or TED gain traction).

> LLMs have millions of man pages, stack overflow answers and shell scripts in their training data. You don't need to teach them how to use your CLI, just show them the `--help`..

LLMs already know how to use CLI tools. I hope `via` can be your bridge while we watch what agents do in IC.

WeChat: 「芯上视图」
