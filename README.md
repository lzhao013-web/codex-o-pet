# codex-o-pet

`codex-o-pet` 是 Codex CLI 与 [o-pet](https://github.com/Orion-zhen/o-pet) 之间的活动桥接插件。它监听 Codex 会话、提示词和工具调用等生命周期事件，并让 o-pet 桌宠播放对应动画。

本项目只负责转发活动状态，不包含 o-pet，也不会自动安装或启动 o-pet。插件不会读取提示词正文、工具参数、工具输出或回复正文。

## 需要配合什么使用

运行本项目必须具备：

- [o-pet](https://github.com/Orion-zhen/o-pet)：负责显示桌宠和动画，需要单独构建并运行。
- Codex CLI：需要支持插件、生命周期 Hooks 和 `mcp_tool` Hook。本项目已使用 Codex CLI `0.149.0` 验证。
- Rust 工具链：用于构建和安装 `codex-o-pet-bridge`。

支持 Windows、Linux 和 macOS。

## 安装

### 1. 安装并启动 o-pet

按照 [o-pet 仓库](https://github.com/Orion-zhen/o-pet)的说明安装。通过源码运行时，可以执行：

```bash
git clone https://github.com/Orion-zhen/o-pet.git
cd o-pet
cargo run --release
```

请保持 o-pet 运行。

### 2. 安装 Bridge

在本项目根目录执行对应脚本。

Windows PowerShell：

```powershell
.\scripts\build-plugin.ps1
```

Linux 或 macOS：

```bash
./scripts/build-plugin.sh
```

脚本会通过 `cargo install --path . --force` 安装 `codex-o-pet-bridge`。启动 Codex 的环境必须能从 `PATH` 找到该命令。

### 3. 安装 Codex 插件

将本项目根目录注册为本地 Marketplace，再安装插件。请把路径替换为实际的项目路径：

```bash
codex plugin marketplace add /path/to/codex-o-pet
codex plugin add codex-o-pet@codex-o-pet-local
```

Codex 首次加载插件 Hooks 时可能要求审核。检查 Hook 定义后按提示信任并启用即可。

## 使用

每次使用时按以下顺序启动：

1. 启动 o-pet。
2. 启动新的 Codex 会话。
3. 正常使用 Codex。桌宠会根据会话和工具调用状态播放动画。

如果 o-pet 没有运行，插件不会中断 Codex 任务。Bridge 会在后续事件中重新尝试连接。

## 工作方式

```text
Codex lifecycle Hooks
  -> codex-o-pet MCP tool
  -> codex-o-pet-bridge
  -> 本地 Socket 或 Windows 命名管道
  -> o-pet
```

可以通过 `O_PET_ENDPOINT` 环境变量覆盖 Bridge 与 o-pet 使用的 IPC 端点。

## 当前限制

- Codex 尚未提供精确的回复开始 Hook，桌宠会在 `Stop` 时播放回复完成动画。
- 失败的工具调用可能保持工具动画，直到下一次状态更新或当前轮次结束。
- 插件只向 o-pet 发送活动状态，不支持通过桌宠批准操作或提交提示词。

## 开源协议

本项目采用 [GNU Affero General Public License v3.0](LICENSE)，与 o-pet 保持一致。

## 开发验证

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```
