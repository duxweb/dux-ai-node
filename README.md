# dux-ai-node

Dux AI 的独立节点运行时，负责客户端注册、状态上报、桌面动作执行与结果回传。

## 当前能力
- 设备注册与 `连接 ID(client_id) / node_token` 回写（注册请求使用设备 ID 作为鉴权标识）
- Runtime WebSocket 长连、心跳、状态上报
- macOS / Windows 托盘入口
- 设置窗口与权限窗口（Rust 内嵌 HTML）
- 浏览器动作：`browser.read`、`browser.screenshot`、`browser.goto`、`browser.extract`、`browser.click`、`browser.type`
- 文件与系统动作：`file.list`、`file.stat`、`file.read_text`、`file.open`、`system.info`、`terminal.exec`
- 截图动作：`screen.capture`
- Artifact 分片上传与 PHP 续跑闭环

## 目录
- `crates/core`：配置、协议、Runtime 长连、artifact 回传
- `crates/browser`：`chromiumoxide` 浏览器控制、截图、文件/系统动作
- `crates/platform`：平台路径、权限与开机自启
- `apps/node-daemon`：Linux daemon 入口
- `apps/node-tray`：macOS / Windows 托盘入口（可执行名 `dux-ai-node`）
- `helpers/macos-ax-helper`：macOS 平台 UI helper（Swift）
- `helpers/windows-uia-helper`：Windows 平台 UI helper 规划目录
- `assets/icon.*`：节点应用图标、托盘图标与平台打包图标资源
- `deploy/systemd/dux-ai-node.service`：Linux 用户态 systemd 模板
- `scripts/build-macos-app.sh`：macOS `.app` 打包
- `scripts/build-linux.sh`：Linux daemon 打包（含跨编译检查）
- `scripts/install-linux.sh`：Debian / Ubuntu 服务器一键安装脚本（默认安装 headless Chromium + systemd 服务）
- `scripts/install-systemd-user.sh`：Linux 用户服务安装
- `scripts/build-windows.ps1`：Windows tray 二进制打包（需在 Windows 主机执行）
- `.github/workflows/build-node.yml`：GitHub Actions 多平台构建与 Release 附件上传
- `scripts/smoke-test-node.sh`：节点动作冒烟测试

## 配置文件
- macOS: `~/Library/Application Support/plus.dux.dux-ai-node/config.toml`
- Windows: `%APPDATA%/plus/dux/dux-ai-node/config.toml`
- Linux: `~/.config/plus/dux/dux-ai-node/config.toml`

关键字段：
- `device_id`：本地稳定 UUID
- `client_id`：服务端注册后返回的连接 ID（当前在线节点标识）
- `node_token`：Runtime WebSocket 鉴权 token

## 常用命令
### Daemon
- `cargo run -p dux-ai-node-daemon -- init`
- `cargo run -p dux-ai-node-daemon -- status`
- `cargo run -p dux-ai-node-daemon -- config get`
- `cargo run -p dux-ai-node-daemon -- config set server_url http://duxai.test`
- `cargo run -p dux-ai-node-daemon -- register`
- `cargo run -p dux-ai-node-daemon -- daemon`

### Tray
- `cargo run -p dux-ai-node -- status`
- `cargo run -p dux-ai-node -- register`
- `cargo run -p dux-ai-node -- run`
- `cargo run -p dux-ai-node -- autostart status`
- `cargo run -p dux-ai-node -- autostart install`
- `cargo run -p dux-ai-node -- autostart uninstall`

## macOS
- 构建 `.app`：`./scripts/build-macos-app.sh`
- 产物：`dist/Dux AI Node.app`
- 启动：`open "dist/Dux AI Node.app"`
- 当前已实现：不进 Dock、菜单栏常驻、设置/权限窗口、LaunchAgent 自启入口

### macOS 提示“已损坏，无法打开”怎么办

如果 macOS 下载后提示：

```text
“Dux AI Node”已损坏，无法打开。你应该将它移到废纸篓。
```

这通常不是文件真的损坏，而是系统对未签名或未公证应用的隔离拦截。

可以在终端执行：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Dux\ AI\ Node.app
```

执行后再重新打开应用即可。

## Linux
- 当前只支持 daemon/headless，不提供 GUI
- `file.open` 等 GUI 相关动作会显式返回不支持
- 构建：`TARGET=x86_64-unknown-linux-gnu ./scripts/build-linux.sh`
- 构建：`TARGET=aarch64-unknown-linux-gnu ./scripts/build-linux.sh`
- 如果在 macOS 上执行该脚本，会先检查 `x86_64-linux-gnu-gcc`，缺失时直接报清晰提示
- 安装用户服务：`./scripts/install-systemd-user.sh`
- Debian / Ubuntu 一键安装：
  - `curl -fsSL https://raw.githubusercontent.com/duxweb/dux-ai-node/main/scripts/install-linux.sh | sudo bash -s -- --server-url http://duxai.test`
  - 重新执行同一个安装命令会自动识别为安装 / 升级 / 重装，并保留现有配置与数据
  - 卸载：`curl -fsSL https://raw.githubusercontent.com/duxweb/dux-ai-node/main/scripts/uninstall-linux.sh | sudo bash`
  - 完全卸载：`curl -fsSL https://raw.githubusercontent.com/duxweb/dux-ai-node/main/scripts/uninstall-linux.sh | sudo bash -s -- --purge`
- 服务模板：`deploy/systemd/dux-ai-node.service`

## Windows
- 当前使用与 macOS 相同的 tray 入口代码
- 构建：在 Windows PowerShell 里运行 `./scripts/build-windows.ps1`
- 该脚本会明确要求在 Windows 主机执行，不再给出误导性的本机跨编译假象

## Smoke Test
- 使用：`./scripts/smoke-test-node.sh <server_url> <device_id> <session_id> <连接ID(client_id)>`
- 会顺序测试：
  - `system.info`
  - `browser.read`
  - `screen.capture`
  - `browser.screenshot`

## GitHub Actions
- 推送 `v*` tag 时会自动构建 Linux x86_64 / arm64 daemon、Windows x86_64 tray、macOS x86_64 / arm64 `.app`
- 构建产物会同时上传到 Actions Artifacts 和 GitHub Release 附件
- 打包结果不再内置 Node.js / Playwright 运行时，当前 macOS 分发基于 Rust + Swift helper

## 说明
- 设置窗口与权限窗口当前由 Rust 内嵌 HTML 提供，不依赖独立前端构建链
- 本仓库不包含 OCR、WPS/微信等通用桌面软件控制实现
