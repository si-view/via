# via

> IC 设计中，**via**（过孔）穿透绝缘层，将信号从一层金属引到另一层——不做变换，只做连接。这个工具的作用与此相同：它在 Virtuoso SKILL 与外部进程之间打通一条 IPC 通道，不改变任何一侧的语义。

`via` 是一个轻量级 IPC 桥接工具，通过 Unix domain socket 将 Cadence Virtuoso SKILL 与外部进程连接起来。任何程序都可以向运行中的 Virtuoso 会话发送 SKILL 表达式，并以 JSON 格式接收求值结果。

## 架构

```
外部进程
    │
    │  via send --eval "dbOpenCellView(...)"
    ▼
┌────────────────��──────────────────────────────┐
│  via serve  (Unix socket，帧格式 JSON)          │
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

| 进程 | 职责 | 启动方 |
|---|---|---|
| `via serve` | 接受客户端连接；串行调度 SKILL 求值；持有两个 socket | Virtuoso `ipcBeginProcess` |
| `via forward` | 将 Virtuoso 输出转发到回调 socket | Virtuoso `ipcBeginProcess` |
| `via send` | 单次客户端：发送表达式，打印 JSON 结果 | Shell / 外部进程 |

## 快速上手

**1. 在 Virtuoso CIW 中加载桥接脚本：**

```skill
load("/path/to/via.il")
si_view_start("/usr/local/bin/via" ?secret "your-secret")
; [si-view] started  pid=12345  bridge=12346  sock=/tmp/via-<user>.sock
```

**2. 从任意 Shell 或进程发送表达式：**

```bash
via send --secret "your-secret" --eval '1 + 1'
# {"id":"…","ok":true,"data":2,"is_ref":false,"code":0}

via send --secret "your-secret" --eval 'getShellEnvVar("HOME")'
# {"id":"…","ok":true,"data":"/home/user","is_ref":false,"code":0}

via send --secret "your-secret" --eval 'dbOpenCellView("myLib" "myCell" "layout")'
# {"id":"…","ok":true,"data":{"id":"cellView:0x7f3a…","kind":"cellView"},"is_ref":true,"code":0}
```

**3. 停止桥接：**

```skill
si_view_stop()
```

## 返回格式

每次 `via send` 调用均返回一个 JSON 对象：

```json
{"id":"…","ok":true, "data":<值>,               "is_ref":false,"code":0}
{"id":"…","ok":false,"data":null,"reason":"…",  "is_ref":false,"code":0}
```

| 字段 | 含义 |
|---|---|
| `ok` | `true` 表示成功，`false` 表示 SKILL 报错 |
| `data` | 求值结果；失败时为 `null` |
| `reason` | 错误描述；仅在 `ok` 为 `false` 时出现 |
| `is_ref` | `true` 表示 `data` 是一个远程对象句柄 |
| `code` | 保留字段，当前始终为 `0` |

### 远程对象

当 SKILL 表达式返回不可直接序列化的对象（如 cellView、db 对象等）时，`via` 将其保存在服务端，并返回一个句柄：

```json
{"id":"…","ok":true,"data":{"id":"cellView:0x7f3a…","kind":"cellView"},"is_ref":true,"code":0}
```

将 `id` 传回后续表达式即可继续操作该对象：

```bash
via send --secret "your-secret" \
         --eval '_via_remote_tbl["cellView:0x7f3a…"]->cellName'
# {"id":"…","ok":true,"data":"myCell","is_ref":false,"code":0}
```

## SKILL API

| 函数 | 说明 |
|---|---|
| `si_view_start(binary_path ?sock ?secret)` | 启动桥接；`?secret` 可选 |
| `si_view_stop()` | 停止桥接 |
| `si_view_emit(name val)` | 推送一个命名事件及 SKILL 值（记录在 via serve 日志中） |

```skill
; 指定自定义 socket 路径
si_view_start("/usr/local/bin/via"
  ?sock   "/tmp/via-myproject.sock"
  ?secret "your-secret")

; 推送事件
si_view_emit("progress" 75)
si_view_emit("status" "done")
```

## CLI 参考

```
via send
  --sock      <路径>    目标 socket          [默认: /tmp/via-$USER.sock]
  --secret    <密钥>    共享密钥
  --eval      <表达式>  要求值的 SKILL 表达式
  --load      <文件>    加载并执行一个 SKILL 文件
  --async               发送后立即退出，不等待结果

via serve
  --sock      <路径>    客户端连接 socket    [默认: /tmp/via-$USER.sock]
  --cb-sock   <路径>    回调 socket          (由 SKILL 设置)
  --cb-token  <令牌>    回调令牌             (由 SKILL 设置)
  --secret    <密钥>    共享密钥             [默认: "" = 不鉴权]
  --log-file  <路径>    日志文件             [默认: via.log]

via forward
  --cb-sock   <路径>    目标回调 socket
  --cb-token  <令牌>    每行前缀令牌
  --log-file  <路径>    日志文件             [默认: via-forward.log]
```

`via serve` 和 `via forward` 由 SKILL 桥接脚本自动管理，通常只需使用 `via send`。

### 示例

```bash
# 求值 SKILL 表达式
via send --secret "s3cr3t" --eval 'geGetEditCellView()'

# 加载 SKILL 文件
via send --secret "s3cr3t" --load /path/to/setup.il

# 指定 socket 路径
via send --sock /tmp/via-myproject.sock --secret "s3cr3t" --eval 'techGetTechFile()'

# 异步发送（不等待结果）
via send --async --secret "s3cr3t" \
         --eval "hiDisplayAppDBox(?name 'hello ?dboxBanner \"via\")"
```

## 与 skillbridge 的对比

[skillbridge](https://github.com/unihd-cag/skillbridge) 是一个成熟的开源项目，用于将 Cadence Virtuoso SKILL 桥接到 Python。它在 Virtuoso 中加载一个 SKILL 服务端脚本（`server.il`），并将 SKILL 函数封装为 Python 代理对象，使调用方可以直接在 Python 中写 `ws.db.open_cell_view(...)` 这样的代码。

`via` 采取了不同的设计思路：

| | skillbridge | via |
|---|---|---|
| **客户端语言** | 仅 Python | 任意语言——Shell、Rust、Python、Go 等均可 |
| **接口形式** | Python 代理对象，封装 SKILL 函数 | 原始 SKILL 表达式字符串 |
| **传输方式** | TCP 或 Unix socket | Unix socket |
| **鉴权** | 无内置鉴权 | 共享密钥（`--secret`） |
| **部署方式** | `pip install skillbridge` + Python 运行时 | 单个静态二进制，无运行时依赖 |
| **异步支持** | 同步 | 同步（默认）或发后不管（`--async`） |
| **返回格式** | Python 对象 | 结构化 JSON（`data`、`ok`、`is_ref`、`code`） |

**适合使用 skillbridge 的场景：** 你在 Python 环境中工作，希望以 Pythonic 的方式调用 SKILL 函数。它非常适合交互式脚本和 Jupyter 工作流。

**适合使用 via 的场景：** 你需要一个语言无关、部署简单的桥接方案——例如将 Virtuoso 集成到非 Python 的工具链、让 SKILL 成为真正的 Agent SKILL， via is all your need!

## 安全

- **`--secret`** 是 `via serve` 与 `via send` 之间的共享密钥。仅在完全隔离的本地环境中可省略。
- 传输通道为 Unix domain socket，访问权限由文件系统权限控制。
- 密钥不得包含空格或 Shell 元字符，推荐使用 32 位十六进制字符串。

## 构建

### 前置依赖

| 工具 | 安装方式 |
|---|---|
| Rust 工具链 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| musl 交叉编译器（Linux 目标） | `brew install musl-cross` |

### 构建全部目标

```bash
./build.sh
```

产物在 `dist/` 目录下：

```
dist/
├── via-linux-x86_64    # ELF x86-64，静态链接，已剥离符号
├── via-linux-aarch64   # ELF aarch64，静态链接，已剥离符号
├── via-macos-x86_64    # Mach-O x86_64
└── via-macos-aarch64   # Mach-O arm64
```

### 构建指定目标

```bash
./build.sh linux-x86_64
./build.sh macos-aarch64
./build.sh --debug linux-x86_64   # 调试构建
```

## 部署

将单个二进制文件复制到目标机器，无需任何运行时依赖：

```bash
scp dist/via-linux-x86_64 user@ic:/usr/local/bin/via
chmod +x /usr/local/bin/via
```

在 Virtuoso 中加载 `via.il` 并调用 `si_view_start` 即可。
