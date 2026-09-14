# 更新日志

Storm Dock 的重要变更记录在此。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

## [1.5.3] - 2026-09-14

### Fixed

- 修复发版构建：`canLaunchGrokBot` 在排除 `grok` 后仍比较 `"grok"`，触发 TypeScript TS2367，导致 `npm run build` 失败。

## [1.5.2] - 2026-09-14

### Added

- 独立 **Grok Bot** 顶栏入口：左侧「账号 / 会话」布局，状态卡片、会话列表与 Cursor 账号列表解耦（新建 `GrokBotAccountList` / `GrokBotSessionWorkspace` / `GrokBotStatusCard`）。
- Grok Build 订阅档位徽章（Free / SuperGrok 等），刷新走 `cli-chat-proxy.grok.com/v1/user?include=subscription`；`subscriptionTier` 为空时记为 Free，不再显示「未知订阅」。
- Grok Build 账号「用量查询」：对接 `cli-chat-proxy.grok.com/v1/billing`，用量页在 `kind=grok` 时使用 Grok 文案与水印（本月额度 / 按需用量 / 近几月）。
- Grok Bot 专用导出 `kind: grok-bot-client`，以及账号列表刷新 `refresh_grok_bot_accounts` / Grok 订阅刷新 `refresh_all_grok_accounts`。

### Fixed

- Cursor ↔ Grok Bot 同应用切换只关闭模式，不再清空账号列表（避免 `selected` 仍为 cursor 时列表被清空且不重载）。
- 顶栏账号刷新仅刷新 Bot 可用账号（非 Free Cursor + Grok Build），会话顶栏刷新只刷新会话；状态卡片刷新当前账号额度与会话。
- 再次点击 Storm Dock 图标时聚焦已有窗口，而不再打开第二个实例。

### Changed

- Grok Build 账号页刷新改为真正拉取订阅；Grok 用量页返回目标为 Grok Build 账号列表。

## [1.5.1] - 2026-09-12

### Fixed

- Windows: 切换 Cursor 账号时不再弹出控制台窗口（用 Win32 API 替代 `tasklist` / `taskkill` / `cmd start`）。
- Cursor 正在运行、需要确认强制重启时，不再提前写入会话或显示切换进度。

## [1.5.0] - 2026-09-10

### Changed

- **发版流水线重写**（对齐 cc-switch 思路，不再打补丁）：
  - 稳定产物命名：`Storm-Dock-<ver>-macOS.*` / `Storm-Dock-<ver>-Windows-x64.*`
  - CI 脚本：`scripts/ci/prepare-signing-key.sh`、`package-macos-assets.sh`、`package-windows-assets.sh`
  - 发布前资产门禁：缺 `.app.tar.gz`/`.sig`/`.dmg` 或 Windows MSI+sig 则失败
  - `latest.json` 强制包含 `darwin-aarch64`（含 `-app`）与 `windows-x86_64`，并 curl 校验公开清单
  - macOS runner 固定 `macos-14`；Release 并发组按 tag 串行
- 文档：`docs/updater.md` 改为完整发版清单

## [1.4.2] - 2026-09-10

### Fixed

- macOS 应用内更新：CI 始终产出并发布已签名的 `.app.tar.gz` 更新包，并在 `latest.json` 中包含 `darwin-aarch64` 平台。

## [1.4.1] - 2026-09-10

### Fixed

- 修复 CI `build.yml` YAML 解析错误（会阻断 v1.4.0 标签工作流）；将 CHANGELOG notes 生成挪到 `scripts/changelog-notes.py`。

## [1.4.0] - 2026-09-10

### Added

- 关闭窗口行为：每次询问、最小化到托盘、或退出（可记住选项），支持 macOS 与 Windows。
- Cursor 账号卡片上的 Grok Bot 用量徽章（百分比 + 短重置文案），以及当前 Grok Bot 账号识别与启动按钮绿色闪电标记。
- Cursor 用量详情中展示 Grok Bot 具体重置时间与相对倒计时。
- 设置 → 关于：应用内检查 / 下载 / 安装更新（Tauri updater + CI `latest.json`）。
- 优化 Windows NSIS/MSI 安装体验：简体中文 + 英文语言选择、安装范围、LZMA 压缩、开始菜单、品牌图、每用户 WiX 模板、WebView2 bootstrapper。
- 美化 macOS DMG 布局（背景、图标位置、最低系统版本 12.0）。

### Changed

- 设置中的窗口行为改为三选一（询问 / 托盘 / 退出）。
- 最小化到托盘时，账号列表与托盘显隐对 Dock / 任务栏状态的恢复更一致。

<!--
发版步骤：
1. 将 Unreleased 条目移入新的 ## [X.Y.Z] - YYYY-MM-DD 小节（默认中文）。
2.  bump package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml 版本号。
3. git tag vX.Y.Z && git push <remote> vX.Y.Z
4. CI 发布安装包与 latest.json；notes 优先取本 CHANGELOG 对应小节。
-->

[Unreleased]: https://github.com/tangsj-hub/Storm-Dock/compare/v1.5.3...HEAD
[1.5.3]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.5.3
[1.5.2]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.5.2
[1.5.1]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.5.1
[1.5.0]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.5.0
[1.4.2]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.4.2
[1.4.1]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.4.1
[1.4.0]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.4.0
[1.3.0]: https://github.com/tangsj-hub/Storm-Dock/releases/tag/v1.3.0
