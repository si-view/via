# via

中文 | **[English](README_en.md)**



<div align="center">
  <img src="https://raw.githubusercontent.com/si-view/via/master/images/logo.png" alt="via — SKILL-IPC bridge" width="480" />

[![crates.io](https://img.shields.io/crates/v/virtuoso-via)](https://crates.io/crates/virtuoso-via)
[![Release](https://github.com/si-view/via/actions/workflows/release.yml/badge.svg)](https://github.com/si-view/via/actions/workflows/release.yml)
[![Platform: Linux](https://img.shields.io/badge/platform-linux-lightgrey)](https://github.com/si-view/via/releases)

</div>

<div align="center">
  <img src="https://raw.githubusercontent.com/si-view/via/master/images/demo.gif" alt="via demo" />
</div>


> IC 设计中，**via** 是将信号从一层金属引到另一层，通常称之为打孔，充当衔接上下两个金属层的作用。这个工具的作用与此相同：它在 Virtuoso SKILL 与外部进程之间打通一条 IPC 通道，上层是应用，下层是 Virtuoso。

`via` 是由 Rust 编写的一个符合 Agent 工学的轻量级(不到 2M) IPC 桥接工具，通过 Unix domain socket 将 Cadence Virtuoso SKILL 与外部进程连接起来。任何程序都可以向运行中的 Virtuoso 会话发送 SKILL 表达式，并以 JSON 格式接收执行结果。

## 架构

```
外部进程
    │
    │  via send --eval "dbOpenCellView(...)"
    ▼
┌───────────────────────────────────────────────┐
│  via serve  (Unix socket，帧格式 JSON)          │
│                                               │
│  router  ──── stdout ─────────────────────── │──► Virtuoso SKILL
│                                               │        │
│  callback reader  ◄── callback socket  ◄───── │────────┘
└───────────────────────────────────────────────┘
         ▲
    via forward（预留，尚无实际通路）
         ▲
    Virtuoso SKILL (si_view_on_data)
```


| 进程            | 职责                                               | 启动方                        |
| ------------- | ------------------------------------------------ | -------------------------- |
| `via serve`   | 接受客户端连接；串行调度 SKILL 求值；持有两个 socket                | Virtuoso `ipcBeginProcess` |
| `via forward` | **占位**：为后续 **Virtuoso 端主动通知**（向外推事件）预留接口；当前无实质作用 | Virtuoso `ipcBeginProcess` |
| `via send`    | 单次客户端：发送表达式，打印 JSON 结果                           | Shell / 外部进程               |


**关于 `via forward`：** 桥接仍会启动该进程以保持 IPC 结构一致，但端到端尚未形成可用能力。它预留给将来封装「Virtuoso → 外部进程」的主动通知，而不改动整体架构形态。

## 五分钟上手

### 0. 安装

```bash
cargo install virtuoso-via
```

> 没有 Rust？先执行：`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
>
> Linux 用户也可以从 [Releases](https://github.com/si-view/via/releases) 下载预编译的静态二进制（无 glibc 依赖）。(支持古董级别操作系统，如 centos/rhel 6.9)

### 1. 启动 Virtuoso 实例

在桌面的终端中，或在指定了`DISPLAY`环境变量的终端中执行：

```bash
via start --name ic # 在当前工作路径启动 virtuoso 服务
via start --name ic --nograph          # 无 GUI 模式
via start --name ic --workspace ~/projects/my_chip  # 指定工作目录
```

> `ic` 是你给 virtuoso 实例启动的别名，你可以开启多个 virtuoso 实例，只需要赋予不同的别名。

如果你知道VNC 启动的端口号，如`:1`

那么可以 `env DISPLAY=:1 via start --name ic`即可在远程终端直接启动。

执行后, Via 在后台启动 Virtuoso，自动注入桥接模块并完成连接。

### 2. 查看运行状态

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

### 3. 发送 SKILL 表达式

```bash
via send --name ic --eval 'geGetEditCellView()'   # 获取当前打开的 cellView
via send --name ic --eval 'hiGetPoint()'           # 执行任意 SKILL
via send --name ic --load ./my_script.il           # 加载 SKILL 文件
via send --name ic --eval 'someHeavyTask()' --async  # fire-and-forget
```

结果以 JSON 返回，可直接接入脚本或工具链：

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "ok": true,
  "data": "schematic",
  "is_ref": false,
  "code": 0
}
```

#### 返回值字段说明


| 字段       | 类型      | 说明                                             |
| -------- | ------- | ---------------------------------------------- |
| `id`     | string  | 请求 UUID，`--async` 模式下可用于追踪                     |
| `ok`     | bool    | `true` = 执行成功；`false` = SKILL 报错或鉴权失败          |
| `data`   | any     | SKILL 返回值序列化后的 JSON；`ok` 为 `false` 时固定为 `null` |
| `reason` | string? | 仅在 `ok: false` 时出现，返回尝试捕获的 error 信息            |
| `is_ref` | bool    | `true` 表示 `data` 是远程对象句柄，而非普通值（见下）             |
| `code`   | int     | 保留字段，当前恒为 `0`                                  |


#### SKILL 类型与 `data` 的对应关系


| SKILL 类型                     | `data` 示例                                                                                                     |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `nil`                        | `null`                                                                                                        |
| `t`                          | `true`                                                                                                        |
| 整数 / 浮点数                     | `42` / `3.14`                                                                                                 |
| 字符串                          | `"schematic"`                                                                                                 |
| symbol                       | `"readOnly"`                                                                                                  |
| list                         | `[1, 2, 3]`                                                                                                   |
| plist                        | 与 **list** 相同：按属性顺序序列化为 JSON **数组**（例如 symbol 与值交替出现，`symbol` 为 `{"__sym":"…"}`）。**不会**把 plist 展开成一个 JSON 对象。 |
| table                        | `{"key1": ..., "key2": ...}`                                                                                  |
| dbObject / cellView 等不可序列化对象 | 远程句柄（`is_ref: true`，见下）                                                                                       |


#### 远程对象句柄（`is_ref: true`）

当返回值是 Virtuoso 内部对象（如 `dbobject`）时，Via 无法将其序列化为 JSON，会改为返回一个句柄，并将对象缓存在 Virtuoso 内存中：

```json
{
  "ok": true,
  "data": { "id": "db:0x1f7fcf1a", "kind": "dbobject" },
  "is_ref": true,
  "code": 0
}
```

后续可通过 `via_remote()` 在 SKILL 侧取回该对象：

- 获取当前打开的 cellview 对象

```bash
via send --name ic --eval 'geGetEditCellView()'
```

输出：

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

- 获取其中的属性值

```bash
via send --name ic --eval 'via_remote("db:0x2113d71a")->cellName'

```

输出

```json
{
  "id": "7c75101b-d7a8-4a8a-bba1-25ab4909c3d1",
  "ok": true,
  "data": "GND",
  "is_ref": false,
  "code": 0
}
```

- 特别的，当想尝试获取一个对象的所有属性：

```bash
via send --name a --eval 'via_remote("db:0x2113d71a")->lib->?' # 获取 lib 的所有字段
```

```json
{
  "id": "6775b54d-41bb-4ac5-8415-5f6885bed458",
  "ok": true,
  "data": [
    {
      "__sym": "type" // symbol 字段会被特殊标记，通过 __sym 标识
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

#### 失败示例

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

`ok: false` 时 `via send` 以非零退出码退出，可在 shell 脚本中直接用 `&&` / `||` 处理。

### 4. 关闭实例

```bash
via kill ic
```

Via 通过 SKILL `exit()` 优雅关闭 Virtuoso，并自动清理 socket 与注册信息。

---

## 进阶用法

### 多实例并行

```bash
via start --name analog  --workspace ~/analog
via start --name digital --workspace ~/digital

via send --name analog  --eval 'getWorkingDir()'
via send --name digital --eval 'getWorkingDir()'
```

### dry-run （不实际执行，即检查运行条件是否具备，并给出反馈）

via 的 部分操作支持 --dry-run 操作

```bash
via start --name ic --dry-run
via kill  ic --dry-run
via send  --name ic --eval 'hiGetPoint()' --dry-run
```

### 清理僵尸实例

```bash
via list --prune
```

### 完整命令参考

```
via start  --name <name> [--workspace <path>] [--virtuoso <bin>] [--nograph] [--dry-run]
via list   [--prune] [--dry-run]
via kill   <name> [--force] [--dry-run]
via send   --name <name> (--eval <expr> | --load <file>) [--async] [--dry-run]
via send   --sock <path> [--secret <s>] (--eval <expr> | --load <file>) [--async]
```

### 与外部工具集成

Via 的 JSON 输出格式适合从任意语言消费，也适合接入 LLM 工具调用（Function Calling）——将 `via send` 封装为工具，即可通过自然语言驱动 Virtuoso 读取版图、查询网表、触发 DRC/LVS 等操作。

**在 IC 圈爆火的 AI 浪潮，via is all you need。**

### 安全性：密钥工作流

Via 使用共享密钥防止未授权进程向 Virtuoso 发送任意 SKILL 代码。

#### 工作原理

执行 `via start` 时，Via 随机生成密钥并写入 `~/.via/registry.json`（仅当前用户可读）。Virtuoso 启动后通过内部 IPC 自动拉起 `via serve` 并同步密钥，两端握手完成。

```
via start
  │
  ├─ 生成随机密钥
  ├─ 写入 ~/.via/registry.json（仅当前用户可读）
  │
  └─ 启动 Virtuoso
        └─ Virtuoso 通过 IPC 自启动 via 守护进程 并同步密钥
              └─ 两端密钥完成同步，桥接就绪

via send --name ic --eval '...'
  └─ 从 registry 读取密钥，随请求发送至 via serve
        └─ via serve 校验匹配 → 执行并返回结果
           via serve 校验失败 → 拒绝连接
```

## 与 skillbridge 的对比

[skillbridge](https://github.com/unihd-cag/skillbridge) 是一个成熟的开源项目，用于将 Cadence Virtuoso SKILL 桥接到 Python。它在 Virtuoso 中加载一个 SKILL 服务端脚本（`server.il`），并将 SKILL 函数封装为 Python 代理对象，使调用方可以直接在 Python 中写 `ws.db.open_cell_view(...)` 这样的代码。

`via` 采取了不同的设计思路：


|           | skillbridge                            | via                                   |
| --------- | -------------------------------------- | ------------------------------------- |
| **客户端语言** | 仅 Python                               | 任意语言——Shell、Rust、Python、Go 等均可        |
| **接口形式**  | Python 代理对象，封装 SKILL 函数                | 原始 SKILL 表达式字符串                       |
| **传输方式**  | TCP 或 Unix socket                      | Unix socket                           |
| **鉴权**    | 无内置鉴权                                  | 共享密钥（`--secret`）                      |
| **部署方式**  | `pip install skillbridge` + Python 运行时 | 单个静态二进制，无运行时依赖                        |
| **异步支持**  | 同步                                     | 同步（默认）或发后不管（`--async`）                |
| **返回格式**  | Python 对象                              | 结构化 JSON（`data`、`ok`、`is_ref`、`code`） |


**适合使用 skillbridge 的场景：** 你在 Python 环境中工作，希望以 Pythonic 的方式调用 SKILL 函数。它非常适合交互式脚本和 Jupyter 工作流。

**适合使用 via 的场景：** 你需要一个语言无关、部署简单的桥接方案——例如将 Virtuoso 集成到非 Python 的工具链、让 SKILL 成为真正的 Agent SKILL，也许，via is all your need!

## 构建

via 目前仅考虑支持 linux、mac操作系统。(考虑到 Virtuoso 本身在 windows 中似乎使用并不广泛，windows 暂时未考虑开发支持)

### 通过 cargo 安装

在 Linux 或 macOS 上最快捷的安装方式：

```bash
cargo install virtuoso-via
```

安装完成后 `via` 二进制会放在 `~/.cargo/bin/`，标准 Rust 安装后该路径已在 `PATH` 中。

### 前置依赖


| 工具                       | 安装方式                                                             |
| ------------------------ | ---------------------------------------------------------------- |
| Rust 工具链                 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` |
| musl 交叉编译器（Linux 目标）（可选） | `brew install musl-cross`                                        |


### 编译

**当前平台（本机 triple）**

```bash
cargo build --release
```

首次执行 `cargo build` 时，Cargo 会从 crates.io 拉取依赖，无需使用 `cargo install`。（可选：仅预取依赖可执行 `cargo fetch`。）

**多架构与 Linux musl**

使用 `./build.sh` 可一次构建 Linux（musl 静态）与 macOS 目标。

```bash
./build.sh                              # 全部目标
./build.sh linux-x86_64                 # 单个或多个目标
./build.sh linux-x86_64 macos-aarch64
./build.sh --debug linux-x86_64         # 调试构建
```

若 musl 交叉编译器不在默认 Homebrew 路径（Apple Silicon：`/opt/homebrew/opt/musl-cross/bin`；Intel：`/usr/local/opt/musl-cross/bin`），可设置：

```bash
export VIA_MUSL_BIN="/你的路径/bin"
./build.sh
```

产物目录默认为项目下的 `dist/`，可用 `VIA_DIST_DIR` 指定其他路径。

产物在 `dist/` 目录下：

```
dist/
├── via-linux-x86_64    # ELF x86-64，静态链接，已剥离符号
├── via-linux-aarch64   # ELF aarch64，静态链接，已剥离符号
```

**Linux 与 glibc：** `via-linux-x86_64`、`via-linux-aarch64` 使用 **musl** 做静态链接，可执行文件**不依赖**目标机上的 **glibc**（也不依赖发行版自带的特定 libc 版本）。因此不会出现「在较新发行版上编译、复制到较老的 RHEL / CentOS 等环境却因 glibc 过旧而无法运行」这类典型问题。在 **CPU 架构一致**（x86_64 或 AArch64）的前提下，同一 Linux 二进制一般可直接用于各主流发行版及其不同版本（仍需内核等环境与权限满足常规可执行文件要求）。  

## 部署

将单个二进制文件复制到目标机器即可，无需额外运行时依赖，这意味着你可以使用更古老的 linux 机器。（Linux 静态 musl 目标见上文）。

**系统级路径（需要 root / sudo）**

```bash
scp dist/via-linux-x86_64 user@ic:/tmp/via
ssh user@ic 'sudo install -m 755 /tmp/via /usr/local/bin/via && rm /tmp/via'
```

也可分步：用 `sudo cp`、`sudo chmod 755` 将二进制放到 `/usr/local/bin/via`（或发行版约定的 `/usr/bin` 等）。

**用户目录（无需 root）**

```bash
mkdir -p ~/.local/bin
scp dist/via-linux-x86_64 user@ic:~/.local/bin/via
ssh user@ic chmod +x ~/.local/bin/via
```

确保登录 shell 的 `PATH` 包含 `~/.local/bin`。例如在 `~/.bashrc` 中加入：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

若使用 csh/tcsh，在 `~/.cshrc` 中加入（与 bash 的 `export` 对应的是 `setenv`）：

```csh
setenv PATH "$HOME/.local/bin:$PATH"
```

## 设计思考

在工程角度上，我认为SKILL的内容，最终都会回到SKILL本身，
我同样也有不少 web工程 相关的开发经验，对于 Virtuoso、SKILL、Bridge, 这很容易让我联想的浏览器、JavaScript、WebAssembly 之间的关系：
在浏览器中的 dom，无论用什么语言去实现 (WebAssembly)，Wasm 本身是无法直接操作 Dom 的，最终操作 Dom 的依然还是 JavaScript，
而类比到SKILL当中，Virtuoso 就是那个浏览器，无论外部如何转换，最终都将SKILL来进行执行。
曾经我对 Python to Skill 的道路也做过尝试，能够解决部分问题，但依然不够。但既然 AI 已经来临，为何不将 SKILL 直接装载到 Agent SKILL，让 AI 理解 SKILL，说 Virtuso 能听得懂的语言开始呢。除非它天然就是 Python (希望能看到 pyAether or TED 发力~)

> LLM 的训练数据里有几百万条 man page、Stack Overflow 回答和 shell 脚本。你的 CLI 不需要教它怎么用，给它看一下 --help 就够了。

LLM 天然就知道如何使用 cli 工具，希望 via 能够作为各位的 bridge，替我看 Agent 在 IC 将会展现何种神力。

公众号：「芯上视图」