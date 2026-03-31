# via

> In IC design, a **via** punches through insulating layers to carry a signal between metal layers — no transformation, just connection. This tool does the same: it bridges Virtuoso SKILL and external processes through IPC without altering the semantics of either side.

`via` is a lightweight IPC bridge that connects [Cadence Virtuoso](https://www.cadence.com/en_US/home/tools/custom-ic-analog-rf-design/circuit-design/virtuoso-studio.html) SKILL to external processes via Unix domain sockets. It lets any program send SKILL expressions into a live Virtuoso session and receive the evaluated result as JSON.

## Architecture

```
External process
    │
    │  via send --expr "dbOpenCellView(...)"
    ▼
┌─────────────────────────────────────────────┐
│  via serve  (Unix socket, framed JSON)       │
│                                             │
│  router  ──── stdout ────────────────────── │──► Virtuoso SKILL
│                                             │        │
│  callback reader  ◄── callback socket  ◄─── │────────┘
└─────────────────────────────────────────────┘
         ▲
         │  stdin → callback socket
    via forward
         ▲
         │  ipcWriteProcess
    Virtuoso SKILL (si_view_on_data)
```

**Three processes, one session:**

| Process | Role | Launched by |
|---|---|---|
| `via serve` | Listens for clients; serialises SKILL evaluation; owns both sockets | Virtuoso (`ipcBeginProcess`) |
| `via forward` | Bridges Virtuoso `ipcWriteProcess` output → callback socket | Virtuoso (`ipcBeginProcess`) |
| `via send` | One-shot client: sends an expression, prints the JSON result | Third-party / shell |

**Why a separate forwarder?**
`ipcBeginProcess` in Virtuoso does not reliably deliver data written via `ipcWriteProcess` to the child's stdin. The forwarder is a dedicated process whose sole job is to receive lines from Virtuoso and relay them to `via serve` over a second Unix socket, working around this limitation entirely.

**Why serial evaluation?**
Virtuoso's SKILL interpreter is single-threaded. The router in `via serve` queues all incoming requests and dispatches them one at a time — only after the previous response has arrived on the callback socket does it send the next expression to stdout.

## IPC Protocol

### Client ↔ Server (Unix socket)

Framing: **4-byte little-endian `u32` length prefix + UTF-8 JSON body.**

```
┌──────────────────┬───────────────────────────────┐
│  4B LE u32 len   │  len bytes — UTF-8 JSON        │
└──────────────────┴───────────────────────────────┘
```

| Direction | JSON |
|---|---|
| `via send` → `via serve` | `{"id":"<uuid>","secret":"…","expression":"…","no_reply":false}` |
| `via serve` → `via send` (success) | `{"id":"<uuid>","status":"success","result":<json>}` |
| `via serve` → `via send` (failure) | `{"id":"<uuid>","status":"failure","error":"…"}` |

### Server ↔ Virtuoso (stdout / callback socket)

| Direction | Format |
|---|---|
| `via serve` stdout → Virtuoso | `SKILL_EXPRESSION\n` |
| Virtuoso → `via forward` stdin | `S:<json>\n` \| `F:<error>\n` \| `E:<json>\n` |
| `via forward` → callback socket | `<CB_TOKEN> S:<json>\n` (token-prefixed) |

## SKILL API

Load `via.il` inside Virtuoso, then:

```skill
load("/path/to/via.il")

; Start the bridge
si_view_start("/usr/local/bin/via"
  ?sock   "/tmp/via-myproject.sock"
  ?secret "your-shared-secret")
; CIW: [si-view] started  pid=12345  bridge=12346  sock=/tmp/via-myproject.sock

; Push an event (logged server-side)
si_view_emit("progress" 75)
si_view_emit("drc:done" list("cell1" "cell2"))

; Stop the bridge
si_view_stop()
```

### Public procedures

| Procedure | Description |
|---|---|
| `si_view_start(binary ?sock ?secret)` | Start `via serve` + `via forward` as Virtuoso child processes |
| `si_view_stop()` | Kill both child processes |
| `si_view_emit(name val)` | Push a named event with a SKILL value (logged server-side) |

### SKILL → JSON type mapping

| SKILL type | JSON |
|---|---|
| `nil` | `null` |
| `t` | `true` |
| integer / float | number |
| string | string |
| symbol `'foo` | `{"__sym":"foo"}` |
| list (even, symbol keys) | `{"key": value, …}` |
| list (other) | `[…]` |
| table | `{"key": value, …}` |
| db object / other opaque | `{"__remote":"_via_remote_N","__kind":"dbObject"}` |

Remote object stubs (`__remote`) are stored as global SKILL variables. Pass the stub back in a later expression to operate on the original object:

```skill
; Returned by via send: {"__remote":"_via_remote_1","__kind":"dbObject"}
; Use in next call:
dbGetCellName(_via_remote_1)
```

## CLI Reference

```
via serve
  --sock      <PATH>    Unix socket for clients   [default: /tmp/via-$USER.sock]
  --cb-sock   <PATH>    Callback socket path      (required, set by SKILL)
  --cb-token  <TOKEN>   Callback auth token       (required, set by SKILL)
  --secret    <SECRET>  Client auth secret        [default: "" = no auth]
  --log-file  <PATH>    Log file                  [default: via.log]

via forward
  --cb-sock   <PATH>    Callback socket path
  --cb-token  <TOKEN>   Token prepended to each forwarded line
  --log-file  <PATH>    Log file                  [default: via-forward.log]

via send
  --sock      <PATH>    Target serve socket       [default: /tmp/via-$USER.sock]
  --secret    <SECRET>  Auth secret
  --expr      <EXPR>    SKILL expression to evaluate
  --async               Fire-and-forget (do not wait for result)
```

### Examples

```bash
# Synchronous — blocks until Virtuoso returns the result
via send --secret "my-secret" \
         --expr 'dbOpenCellView("myLib" "myCell" "layout")'
# stdout: {"__remote":"_via_remote_1","__kind":"dbObject"}

# Use the returned remote ref in a follow-up call
via send --secret "my-secret" \
         --expr 'dbGetCellName(_via_remote_1)'
# stdout: "myCell"

# Asynchronous — fire-and-forget, exits immediately
via send --async --secret "my-secret" \
         --expr 'hiDisplayAppDBox(?title "hello" ?dboxBanner "via")'

# Custom socket
via send --sock /tmp/via-myproject.sock \
         --secret "my-secret" \
         --expr 'car(geGetEditCellView())'
```

## Security

- **`--secret`** is a static shared secret between `via serve` and `via send` callers. Transport is a Unix domain socket (filesystem permissions apply).
- **`--cb-token`** is a per-session random 32-character hex token generated by SKILL in a `let`-local variable — it is never stored in a global and cannot be read by other SKILL code after `si_view_start` returns.
- Set `--secret ""` (the default) only in isolated development environments.
- The secret must not contain spaces or shell metacharacters; a hex string is recommended.

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
./build.sh linux-aarch64
./build.sh macos-aarch64
./build.sh macos-x86_64
```

### Debug build

```bash
./build.sh --debug linux-x86_64
```

### Manual build (single target)

```bash
cargo build --release --target x86_64-unknown-linux-musl
# output: target/x86_64-unknown-linux-musl/release/via
```

### Verifying static linkage

```bash
file dist/via-linux-x86_64
# via-linux-x86_64: ELF 64-bit LSB pie executable, x86-64, static-pie linked, stripped
```

The Linux builds have no `glibc` dependency and run on any Linux kernel ≥ 3.2.

## Deployment

Copy the single binary to the target machine — no runtime dependencies required:

```bash
scp dist/via-linux-x86_64 user@eda-server:/usr/local/bin/via
chmod +x /usr/local/bin/via
```

Then load `via.il` in Virtuoso and call `si_view_start`.
