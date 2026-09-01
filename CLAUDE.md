# SSH Tunnel

跨平台 SSH 端口转发桌面工具。Tauri 2（Rust + russh）+ Vue 3 + Element Plus。
设计文档：`docs/superpowers/specs/2026-09-01-ssh-tunnel-design.md`，实现前必读。

## 目录结构约定

```
src/                  # Vue 前端
  views/              # 页面级组件（MainView、SettingsView）
  components/         # 可复用组件（对话框、日志面板）
  stores/             # Pinia store，按领域分文件
src-tauri/src/        # Rust 后端
  ssh/                # SSH 连接 actor 与 russh client
  forward/            # -L / -R / -D 三种转发实现
  config.rs           # 配置读写（不含敏感值）
  secrets.rs          # 钥匙串封装
  tray.rs             # 托盘
  commands.rs         # 全部 Tauri commands
  error.rs            # 统一错误类型
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
cd src-tauri && cargo test   # 后端测试
pnpm test             # 前端测试（Vitest）
```

## 部署

桌面应用，不涉及局域网测试服务器部署。打包走 GitHub Actions（仓库有远端后配置）。
