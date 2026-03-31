# via

> In IC design, a **via** punches through insulating layers to carry a signal between metal layers — no transformation, just connection. This tool does the same: it bridges Virtuoso SKILL and external processes through IPC without altering the semantics of either side.

`via` is a lightweight IPC bridge that connects [Cadence Virtuoso](https://www.cadence.com/en_US/home/tools/custom-ic-analog-rf-design/circuit-design/virtuoso-studio.html) SKILL to external processes via Unix domain sockets. Any program can send a SKILL expression into a live Virtuoso session and receive the result as JSON.

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
    via forward
         ▲
    Virtuoso SKILL (si_view_on_data)
```

| Process | Role | Launched by |
|---|---|---|
| `via serve` | Accepts clients; queues SKILL evaluation; owns both sockets | Virtuoso `ipcBeginProcess` |
| `via forward` | Relays Virtuoso output to the callback socket | Virtuoso `ipcBeginProcess` |
| `via send` | One-shot client: sends an expression, prints the JSON result | Shell / external process |

## Quick Start

**1. Load the bridge in Virtuoso CIW:**

```skill
load("/path/to/via.il")
si_view_start("/usr/local/bin/via" ?secret "your-secret")
; [si-view] started  pid=12345  bridge=12346  sock=/tmp/via-<user>.sock
```

**2. Send expressions from any shell or process:**

```bash
via send --secret "your-secret" --eval '1 + 1'
# {"id":"…","ok":true,"data":2,"is_ref":false,"code":0}

via send --secret "your-secret" --eval 'getShellEnvVar("HOME")'
# {"id":"…","ok":true,"data":"/home/user","is_ref":false,"code":0}

via send --secret "your-secret" --eval 'dbOpenCellView("myLib" "myCell" "layout")'
# {"id":"…","ok":true,"data":{"id":"cellView:0x7f3a…","kind":"cellView"},"is_ref":true,"code":0}
```

**3. Stop the bridge:**

```skill
si_view_stop()
```

## Response Format

Every `via send` call returns a JSON object:

```json
{"id":"…","ok":true, "data":<value>,        "is_ref":false,"code":0}
{"id":"…","ok":false,"data":null,"reason":"…","is_ref":false,"code":0}
```

| Field | Meaning |
|---|---|
| `ok` | `true` = success, `false` = SKILL error |
| `data` | The result value; `null` on failure |
| `reason` | Error description; only present when `ok` is `false` |
| `is_ref` | `true` when `data` is a remote-object handle |
| `code` | Reserved for future use; always `0` |

### Remote objects

When a SKILL expression returns an opaque object (cell view, db object, etc.), `via` stores it server-side and returns a handle:

```json
{"id":"…","ok":true,"data":{"id":"cellView:0x7f3a…","kind":"cellView"},"is_ref":true,"code":0}
```

Pass the `id` back in a subsequent expression to work with the object:

```bash
via send --secret "your-secret" \
         --eval '_via_remote_tbl["cellView:0x7f3a…"]->cellName'
# {"id":"…","ok":true,"data":"myCell","is_ref":false,"code":0}
```

## SKILL API

| Procedure | Description |
|---|---|
| `si_view_start(binary_path ?sock ?secret)` | Start the bridge; `?secret` is optional |
| `si_view_stop()` | Stop the bridge |
| `si_view_emit(name val)` | Push a named event with a SKILL value (logged server-side) |

```skill
; Custom socket path
si_view_start("/usr/local/bin/via"
  ?sock   "/tmp/via-myproject.sock"
  ?secret "your-secret")

; Emit events (logged on the via serve side)
si_view_emit("progress" 75)
si_view_emit("status" "done")
```

## CLI Reference

```
via send
  --sock      <PATH>    Target socket       [default: /tmp/via-$USER.sock]
  --secret    <SECRET>  Shared secret
  --eval      <EXPR>    SKILL expression to evaluate
  --load      <FILE>    Load and execute a SKILL file
  --async               Fire-and-forget; exit without waiting for a result

via serve
  --sock      <PATH>    Socket for clients  [default: /tmp/via-$USER.sock]
  --cb-sock   <PATH>    Callback socket     (set by SKILL)
  --cb-token  <TOKEN>   Callback token      (set by SKILL)
  --secret    <SECRET>  Shared secret       [default: "" = no auth]
  --log-file  <PATH>    Log file            [default: via.log]

via forward
  --cb-sock   <PATH>    Callback socket to forward to
  --cb-token  <TOKEN>   Token prepended to each line
  --log-file  <PATH>    Log file            [default: via-forward.log]
```

`via serve` and `via forward` are managed by the SKILL bridge — you typically only interact with `via send`.

### Examples

```bash
# Evaluate an expression
via send --secret "s3cr3t" --eval 'geGetEditCellView()'

# Load a SKILL file
via send --secret "s3cr3t" --load /path/to/setup.il

# Custom socket
via send --sock /tmp/via-myproject.sock --secret "s3cr3t" --eval 'techGetTechFile()'

# Fire-and-forget (no result needed)
via send --async --secret "s3cr3t" \
         --eval "hiDisplayAppDBox(?name 'hello ?dboxBanner \"via\")"
```

## Comparison with skillbridge

[skillbridge](https://github.com/unihd-cag/skillbridge) is a well-established open-source project that bridges Cadence Virtuoso SKILL to Python. It loads a SKILL server (`server.il`) into Virtuoso and exposes SKILL functions as Python proxy objects, so callers can write `ws.db.open_cell_view(...)` directly in Python.

`via` takes a different approach:

| | skillbridge | via |
|---|---|---|
| **Client language** | Python only | Any — shell, Rust, Python, Go, … |
| **Interface** | Python proxy objects wrapping SKILL functions | Raw SKILL expression strings |
| **Transport** | TCP or Unix socket | Unix socket |
| **Authentication** | None built-in | Shared secret (`--secret`) |
| **Deployment** | `pip install skillbridge` + Python runtime | Single static binary, no runtime |
| **Async** | Synchronous | Synchronous (default) or fire-and-forget (`--async`) |
| **Response** | Python objects | Structured JSON (`data`, `ok`, `is_ref`, `code`) |

**When to use skillbridge:** You are working in Python and want to call SKILL functions with a natural, Pythonic API. It is the right choice for interactive scripting and Jupyter workflows.

**When to use via:** You need a language-agnostic, deployment-friendly bridge — for example, integrating Virtuoso into a non-Python pipeline, a CI script, or a compiled service. The single static binary and structured JSON responses make it straightforward to embed in any toolchain.

## Security

- **`--secret`** is a shared secret between `via serve` and `via send` callers. Omit it only in isolated local environments.
- Transport is a Unix domain socket — filesystem permissions control who can connect.
- The secret must not contain spaces or shell metacharacters; a 32-character hex string is recommended.

## Build

### Prerequisites

| Tool | Install |
|---|---|
| Rust toolchain | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| musl cross-compiler (Linux targets) | `brew install musl-cross` |

### Build all targets

```bash
./build.sh
```

Output in `dist/`:

```
dist/
├── via-linux-x86_64    # ELF x86-64, static-pie, stripped
├── via-linux-aarch64   # ELF aarch64, static-pie, stripped
├── via-macos-x86_64    # Mach-O x86_64
└── via-macos-aarch64   # Mach-O arm64
```

### Build a specific target

```bash
./build.sh linux-x86_64
./build.sh macos-aarch64
./build.sh --debug linux-x86_64   # debug build
```

## Deployment

Copy the binary to the target machine — no runtime dependencies:

```bash
scp dist/via-linux-x86_64 user@eda-server:/usr/local/bin/via
chmod +x /usr/local/bin/via
```

Load `via.il` in Virtuoso and call `si_view_start`.
