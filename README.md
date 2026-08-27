# codex-o-pet

`codex-o-pet` 是 Codex CLI 与 [o-pet](https://github.com/Orion-zhen/o-pet) 之间的活动桥接插件。它监听 Codex 会话、提示词、工具调用、子代理和上下文压缩等生命周期事件，并让 o-pet 桌宠播放对应动画。

本项目只转发活动状态，不包含 o-pet，也不会读取提示词正文、工具参数、工具输出或回复正文。

## 需要配合什么使用

- [o-pet](https://github.com/Orion-zhen/o-pet)：负责显示桌宠和动画，需要单独安装并运行。
- Codex CLI：需要支持插件、生命周期 Hooks 和 `mcp_tool` Hook。本项目已使用 Codex CLI `0.150.1` 验证。

发布版插件已包含 Bridge，不需要安装 Rust，也不需要手动执行 `cargo install`。支持 Windows x86_64、Linux x86_64/ARM64，以及 macOS Intel/Apple Silicon。

## 安装

### 1. 安装并启动 o-pet

按照 [o-pet 仓库](https://github.com/Orion-zhen/o-pet)的说明安装。通过源码运行时，可以执行：

```bash
git clone https://github.com/Orion-zhen/o-pet.git
cd o-pet
cargo run --release
```

请保持 o-pet 运行。

### 2. 安装 Codex 插件

Linux 或 macOS：

```bash
codex plugin marketplace add lzhao013-web/codex-o-pet --ref plugin-dist \
  && codex plugin add codex-o-pet@codex-o-pet
```

Windows PowerShell：

```powershell
codex plugin marketplace add lzhao013-web/codex-o-pet --ref plugin-dist
if ($LASTEXITCODE -eq 0) { codex plugin add codex-o-pet@codex-o-pet }
```

Codex 首次加载插件 Hooks 时可能要求审核。检查 Hook 定义后按提示信任并启用即可。

## 使用

每次使用时按以下顺序启动：

1. 启动 o-pet。
2. 启动新的 Codex 会话。
3. 正常使用 Codex。桌宠会根据会话和工具调用状态播放动画。

如果 o-pet 没有运行，插件不会中断 Codex 任务。Bridge 会为每个 Codex 会话在内存中保留最近 256 条未发送的状态事件，并在后续 Hook 触发时重新连接和按顺序补发。一个 Bridge 进程最多同时保留 16 个会话连接；超过上限时会关闭并淘汰最久未使用的会话。

## 工作方式

```text
Codex lifecycle Hooks
  -> codex-o-pet MCP tool
  -> 插件内置的 codex-o-pet-bridge
  -> 本地 Socket 或 Windows 命名管道
  -> o-pet
```

插件会将 `XDG_RUNTIME_DIR`、`O_PET_ENDPOINT` 和 `O_PET_LOG` 从 Codex 的本地环境传递给 Bridge。Linux 默认使用 `$XDG_RUNTIME_DIR/o-pet.sock`；也可以通过 `O_PET_ENDPOINT` 环境变量覆盖 Bridge 与 o-pet 使用的 IPC 端点。

当前 Hook 映射包括：

- 会话和提示词活动。
- 本地命令、文件修改、MCP 等可观察工具的开始与结束；Bridge 只根据工具名将读取、搜索、写入、下载、终端和规划类工具映射到更具体的动画。
- 子代理的实际运行周期；启动子代理的短暂工具调用会被去重，探索和审查类子代理显示为搜索，执行类子代理显示为编码，其他类型显示为咨询。
- 手动或自动上下文压缩，使用 o-pet 的 `skill` 工具动画表示。

Bridge 的 Hook 输入采用按事件区分的严格字段集合。提示词正文、工具参数、工具输出以及回复正文都不在允许字段中，意外传入时会在 Bridge 边界被拒绝。

如需排查未识别的工具动画，可以在启动 Codex 前设置 `O_PET_LOG=debug`。诊断日志只输出 Hook 类型、工具名或子代理类型以及映射后的事件数量，不输出会话 ID、调用 ID 或任何内容正文。

## 发布

推送与 `Cargo.toml` 和插件清单版本一致的 `vX.Y.Z` 标签后，`.github/workflows/release.yml` 会：

- 验证格式、Clippy 和测试。
- 构建各平台 Bridge。
- 创建 GitHub Release 及独立平台产物。
- 生成包含全部平台 Bridge 的通用插件。
- 将可直接安装的 Marketplace 发布到 `plugin-dist` 分支。

## 当前限制

- Codex 尚未提供精确的回复开始 Hook，桌宠会在 `Stop` 时播放回复完成动画。
- Hook 没有统一的工具或子代理结果字段；插件会把已结束的生命周期报告为 `success`，因此可能缺少错误动画。
- Codex 托管的工具（例如 `WebSearch`）不经过本地工具 Hook，插件无法观察其开始和结束。
- 插件只向 o-pet 发送活动状态，不支持通过桌宠批准操作或提交提示词。

## 本地开发

源码构建需要 Rust 工具链：

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

如需通过源码 Marketplace 测试插件，先安装开发版 Bridge：

```bash
cargo install --path . --force
codex plugin marketplace add /path/to/codex-o-pet
codex plugin add codex-o-pet@codex-o-pet-local
```

## 开源协议

本项目采用 [GNU Affero General Public License v3.0](LICENSE)，与 o-pet 保持一致。
