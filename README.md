# codex-o-pet

`codex-o-pet` 是 Codex CLI 与 [o-pet](https://github.com/Orion-zhen/o-pet) 之间的活动桥接插件。它监听 Codex 会话、提示词和工具调用等生命周期事件，并让 o-pet 桌宠播放对应动画。

本项目只转发活动状态，不包含 o-pet，也不会读取提示词正文、工具参数、工具输出或回复正文。

## 需要配合什么使用

- [o-pet](https://github.com/Orion-zhen/o-pet)：负责显示桌宠和动画，需要单独安装并运行。
- Codex CLI：需要支持插件、生命周期 Hooks 和 `mcp_tool` Hook。本项目已使用 Codex CLI `0.149.0` 验证。

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

仓库为私有状态时，Git 必须拥有该仓库的读取权限。Codex 首次加载插件 Hooks 时可能要求审核。检查 Hook 定义后按提示信任并启用即可。

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
  -> 插件内置的 codex-o-pet-bridge
  -> 本地 Socket 或 Windows 命名管道
  -> o-pet
```

可以通过 `O_PET_ENDPOINT` 环境变量覆盖 Bridge 与 o-pet 使用的 IPC 端点。

## 发布

推送与 `Cargo.toml` 和插件清单版本一致的 `vX.Y.Z` 标签后，`.github/workflows/release.yml` 会：

- 验证格式、Clippy 和测试。
- 构建各平台 Bridge。
- 创建 GitHub Release 及独立平台产物。
- 生成包含全部平台 Bridge 的通用插件。
- 将可直接安装的 Marketplace 发布到 `plugin-dist` 分支。

## 当前限制

- Codex 尚未提供精确的回复开始 Hook，桌宠会在 `Stop` 时播放回复完成动画。
- 失败的工具调用可能保持工具动画，直到下一次状态更新或当前轮次结束。
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
