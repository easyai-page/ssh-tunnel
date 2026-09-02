# SSH Tunnel

跨平台 SSH 端口转发桌面工具。Tauri 2（Rust + russh）+ Vue 3 + Element Plus。
设计文档：`docs/superpowers/specs/2026-09-01-ssh-tunnel-design.md`，实现前必读。

## 目录结构约定

```
src/                  # Vue 前端
  views/              # 页面级组件（MainView、SettingsView）
  components/         # 可复用组件（对话框、日志面板）
  stores/             # Pinia store，按领域分文件
core/src/             # ssh-tunnel-core：纯 Rust 核心，零 GUI 依赖
  ssh/                # SSH 连接 actor 与 russh client
  forward/            # -L / -R / -D 三种转发实现
  model.rs            # 数据模型
  config.rs           # 配置读写（不含敏感值）
  secrets.rs          # 钥匙串抽象（trait + keyring 实现 + 内存实现）
  known_hosts.rs      # host key 记录
  error.rs            # 统一错误类型
src-tauri/src/        # ssh-tunnel-app：Tauri 壳，依赖 core
  commands.rs         # 全部 Tauri commands
  tray.rs             # 托盘
  logging.rs          # 日志
docs/superpowers/     # spec 与实现计划
```

## 硬性规则

- **敏感值（密码、密钥内容、passphrase）只进系统钥匙串**，配置/日志/错误消息中禁止出现
- 一条 SSH 连接多路复用该服务器所有隧道，禁止一隧道一连接
- 注释解释「为什么」，用中文；代码与命名用英文
- 改完必跑验证：`cd src-tauri && cargo test`，前端 `pnpm type-check`

## 常用命令

```bash
pnpm install          # 装前端依赖
pnpm tauri dev        # 开发模式
cargo test -p ssh-tunnel-core   # 后端核心测试（无 GUI 依赖，可无头运行）
pnpm test             # 前端测试（Vitest）
pnpm type-check       # vue-tsc
```

Tauri 壳（src-tauri）编译需要系统依赖：`sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev`。

## 部署

桌面应用，不涉及局域网测试服务器部署。打包走 GitHub Actions（`.github/workflows/ci.yml`，仓库 `easyai-page/ssh-tunnel`）：
- `test` 阶段：前端 Vitest + vue-tsc、后端 `cargo test -p ssh-tunnel-core`
- `build` 阶段：六平台 Tauri 打包（Windows/Linux/macOS 各 x86_64 + arm64），产物在各 job 的 Artifacts 里
- 触发：push 到 master、PR、手动 `workflow_dispatch`（这三种只编译验证，产物在 Artifacts）
- 发版：push `v*` tag（如 `git tag v0.1.0 && git push origin v0.1.0`），tauri-action 自动创建 GitHub Release 并挂六平台安装包
- 注意：Windows/Linux 的 arm64 免费 runner 仅公开仓库可用，仓库需保持 public
