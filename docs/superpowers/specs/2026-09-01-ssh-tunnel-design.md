# SSH Tunnel 桌面工具 — 设计文档

日期：2026-09-01
状态：已确认（用户批准）

## 1. 概述

一个可视化的跨平台 SSH 端口转发桌面工具。核心使用场景：用户常驻系统托盘，通过右键菜单快速启停已配置的隧道；需要配置时打开主窗口管理服务器和转发规则。

### 功能需求

1. 多 SSH 服务器管理（增删改查）
2. 三种认证方式：密码、密钥文件、粘贴密钥字符串；用户名/密码可保存
3. 三种转发类型：本地转发 -L、远程转发 -R、动态转发 -D（SOCKS5）
4. 隧道随时增删、随时启停，无需重启连接
5. 最小化到系统托盘；托盘右键菜单可启停/增删隧道
6. 跨平台：Windows、macOS、Linux

### 非目标（YAGNI，本版不做）

- 跳板机 / ProxyJump
- SFTP 文件传输
- 内置终端
- 配置云同步

数据模型与架构为上述功能预留扩展空间，但不实现。

## 2. 技术栈

| 层 | 选型 | 理由 |
|---|---|---|
| 桌面框架 | Tauri 2 | 安装包小（~10MB）、内存占用低、系统托盘一等公民 |
| SSH 协议 | russh（纯 Rust async） | 无 C 依赖利于交叉编译；async 多路复用是一条连接挂多条隧道的最佳模型 |
| 前端 | Vue 3 + Pinia + Element Plus | 用户熟悉 Vue；Element Plus 表格/表单组件开箱即用 |
| 凭据存储 | keyring crate | Windows Credential Manager / macOS Keychain / Linux Secret Service |
| 构建 | Vite + tauri-build；GitHub Actions 出三平台包 | — |

曾被否决的方案：Electron（包体过大、常驻内存高）、Go+Fyne（UI 表现力弱）、Python+PySide6（打包痛苦）、ssh2 crate（同步 API 不适合多隧道）、调系统 ssh 命令（进程管理脆弱、UX 不可控）。

## 3. 架构

```
┌─ Vue 前端 ───────────────────────────────┐
│ 服务器列表 │ 转发管理 │ 日志 │ 设置       │
└────── Tauri commands ↑  ↓ events ────────┘
┌─ Rust 后端 ──────────────────────────────┐
│ config   配置读写（JSON，不含敏感值）     │
│ secrets  钥匙串封装（keyring crate）      │
│ ssh      每服务器一个连接 actor（russh）  │
│ forward  -L / -R / -D 三种转发实现        │
│ tray     托盘菜单 + 状态图标              │
└───────────────────────────────────────────┘
```

核心模型：**一条 SSH 连接多路复用该服务器的所有隧道**。

每个服务器对应一个后台 actor（tokio task + mpsc channel）。actor 接收指令（加隧道 / 删隧道 / 断开 / 重连），内部维护 russh session 与各隧道的 listener/forward 句柄。状态变化通过 Tauri event 广播给前端，并触发托盘菜单与图标重建。

### 3.1 后端模块（Rust，src-tauri/src/ 下）

- `config.rs` — 配置的加载、保存、迁移。配置文件为 JSON，敏感字段只存钥匙串引用 key
- `secrets.rs` — keyring 封装。key 命名规范：`ssh-tunnel:<serverId>:password`、`ssh-tunnel:<serverId>:key`、`ssh-tunnel:<serverId>:key-passphrase`
- `ssh/mod.rs` — `SshManager`：持有所有 `ServerActor` 的句柄，路由指令；定义对外事件类型
- `ssh/actor.rs` — `ServerActor`：连接状态机（Disconnected → Connecting → Connected → Reconnecting → Error），自动重连逻辑
- `ssh/client.rs` — russh client 实现：认证（密码 / 密钥文件 / 密钥字符串 + passphrase）、host key 校验
- `forward/local.rs` — 本地转发：起 TcpListener，accept 后开 direct-tcpip channel 桥接
- `forward/remote.rs` — 远程转发：请求 tcpip-forward，收到 forwarded-tcpip 后桥接到本地目标
- `forward/socks.rs` — 动态转发：本地起 SOCKS5 server，目标连接走 direct-tcpip
- `tray.rs` — 托盘图标三态与右键菜单构建；菜单事件分发
- `commands.rs` — 全部 Tauri commands（见 3.3）
- `error.rs` — 统一错误类型，thiserror 定义，错误消息面向用户可读

### 3.2 前端结构（src/）

- `views/MainView.vue` — 左侧服务器列表 + 右侧隧道表格
- `views/SettingsView.vue` — 开机自启、最小化到托盘、自动重连开关、日志级别
- `components/ServerEditorDialog.vue` — 服务器编辑对话框，三个认证 tab（密码 / 密钥文件 / 粘贴密钥）
- `components/ForwardEditorDialog.vue` — 转发编辑（类型切换时动态显示字段）
- `components/LogPanel.vue` — 日志面板
- `stores/servers.ts`、`stores/forwards.ts`、`stores/logs.ts` — Pinia store，监听后端事件保持同步

### 3.3 前后端接口

**Commands**（UI → 后端）：

```
// 服务器 CRUD
list_servers() -> Vec<Server>
upsert_server(server: ServerInput) -> Server
delete_server(id: String)

// 转发 CRUD
list_forwards(server_id: String) -> Vec<Forward>
upsert_forward(f: ForwardInput) -> Forward
delete_forward(id: String)

// 运行时控制
start_forward(id: String) / stop_forward(id: String)
connect_server(id: String) / disconnect_server(id: String)

// 其他
get_logs() -> Vec<LogEntry>
get_settings() / save_settings(s: Settings)
```

**Events**（后端 → UI/托盘）：

```
server-status  { serverId, status, error? }   // disconnected/connecting/connected/reconnecting/error
forward-status { forwardId, status, error? }  // stopped/starting/running/error
log            { level, message, timestamp }
```

### 3.4 数据模型

```rust
struct Server {
    id: String,            // uuid
    name: String,
    host: String,
    port: u16,             // 默认 22
    username: String,
    auth: AuthMethod,
}

enum AuthMethod {
    Password,              // 密码存钥匙串
    KeyFile { path: String },  // passphrase（如有）存钥匙串
    KeyData,               // 密钥内容存钥匙串，不落盘
}

struct Forward {
    id: String,
    server_id: String,
    name: String,
    kind: ForwardKind,
    bind_addr: String,     // 监听地址，默认 127.0.0.1
    bind_port: u16,
    target_host: Option<String>,  // dynamic 类型无目标
    target_port: Option<u16>,
    auto_start: bool,      // 应用启动时自动开启
}

enum ForwardKind { Local, Remote, Dynamic }
```

### 3.5 托盘

- **图标三态**：灰（无活动隧道）/ 绿（活动隧道全部正常）/ 红（任一隧道或连接出错）
- **右键菜单结构**：

```
▸ 服务器A  (状态)
   ✓ 转发1  (local :3306 → db:3306)
     转发2  (dynamic :1080)
   ─────────
   添加转发…
▸ 服务器B
   …
─────────
显示主窗口
退出
```

- 点击隧道项即切换启停；「添加转发…」「显示主窗口」唤起主窗口并定位到对应上下文

## 4. 关键行为

1. **自动重连**：连接断开后指数退避重连（1s 起步，×2，30s 封顶），重连成功后自动恢复该服务器所有处于 running 态的隧道。设置里可全局关闭
2. **端口冲突**：本地端口被占用时隧道状态置 error，消息指明「本地端口 3306 被占用」，不影响其他隧道
3. **认证失败**：状态置 error 并提示原因；密钥带 passphrase 且钥匙串中没有时，前端弹窗询问（可选择保存）
4. **host key 校验**：首次连接记录 host key（known_hosts 风格存配置目录）；变更时弹窗警告，用户确认后才更新
5. **关闭主窗口**：默认最小化到托盘而非退出；「退出」只从托盘菜单触发
6. **开机自启**：tauri-plugin-autostart，设置中开关，默认关

## 5. 凭据与安全

- 密码、粘贴的密钥内容、密钥 passphrase **只进系统钥匙串**，配置文件仅存引用 key
- 密钥文件方式只保存路径，不复制内容
- 配置文件权限 0600（Unix）
- 日志中不输出凭据；错误消息不含敏感值
- 依赖 keyring crate；Linux 需 Secret Service（gnome-keyring 或 KWallet），无桌面环境时启动报错并给出明确提示

## 6. 配置与日志位置

| 平台 | 路径 |
|---|---|
| Linux | `~/.config/ssh-tunnel/` |
| macOS | `~/.config/ssh-tunnel/`（保持与 Linux 一致，不用 Application Support） |
| Windows | `%APPDATA%\ssh-tunnel\` |

目录内容：`config.json`（服务器/转发/设置）、`known_hosts`、`logs/ssh-tunnel.log`（滚动，保留 5 个 × 2MB）。

## 7. 测试策略

- **Rust 单测**：config 读写与迁移、secrets key 命名、SOCKS5 握手、数据模型序列化
- **Rust 集成测试**：测试进程内自起 russh server，客户端真实建连，验证 -L/-R/-D 三种转发端到端通数据；模拟断线验证自动重连与隧道恢复
- **前端**：Vitest 测 store 逻辑与关键组件（ForwardEditorDialog 的字段联动）
- **手动验证矩阵**：Windows 11 / macOS / Linux（GNOME + AppIndicator 扩展）三平台各跑一遍核心流程
- 覆盖率目标 80%（后端核心模块）

## 8. 打包发布

- GitHub Actions 三平台矩阵构建：Windows（msi）、macOS（dmg，universal）、Linux（AppImage + deb）
- 本仓库阶段只到「本地能跑 + 测试通过」；CI 出包在仓库有远端后配置

## 9. 风险与对策

| 风险 | 对策 |
|---|---|
| Linux 托盘兼容性（GNOME 默认无托盘） | 文档说明需 AppIndicator 扩展；用 libappindicator 后端 |
| russh API 变动（crate 较年轻） | 锁定版本，集成测试覆盖真实建连 |
| Linux 无 Secret Service 环境 | 启动检测，明确报错提示安装 gnome-keyring |
| 高并发隧道性能 | 每隧道独立 task，压测留到性能问题时再做（YAGNI） |
