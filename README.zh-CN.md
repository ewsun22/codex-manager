# Codex Manager

[English](README.md) | **简体中文**

> 你的本地 OpenAI Codex 控制中心。

Codex Manager 是一个非官方、本地优先的桌面应用，用于在本机统一了解和管理 Codex 使用情况：session、模型、推理等级、token、账户、供应商、项目和 `AGENTS.md`。

[![最新版本](https://img.shields.io/github/v/release/ewsun22/codex-manager?display_name=tag)](https://github.com/ewsun22/codex-manager/releases/latest) [![CI](https://github.com/ewsun22/codex-manager/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ewsun22/codex-manager/actions/workflows/ci.yml) [![macOS 13+](https://img.shields.io/badge/macOS-13%2B-111827?logo=apple)](https://github.com/ewsun22/codex-manager/releases/latest) [![Tauri 2](https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri)](https://tauri.app/) [![Rust](https://img.shields.io/badge/Rust-1.98%2B-000000?logo=rust)](https://www.rust-lang.org/) [![许可证 Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

**[下载 macOS 版（Apple 芯片）→](https://github.com/ewsun22/codex-manager/releases/latest)**

![Codex Manager 总览仪表盘](output/playwright/desktop-overview.png)

## 下载

**[下载最新签名 macOS 正式版](https://github.com/ewsun22/codex-manager/releases/latest)** · [Release 资产与校验和](https://github.com/ewsun22/codex-manager/releases/latest) · [全部 Releases](https://github.com/ewsun22/codex-manager/releases)

最新的非 draft、非 prerelease GitHub Release 是稳定 macOS arm64 下载的唯一事实来源。每个版本都必须独立通过 Developer ID 签名、Apple 公证、stapling 和公开资产复验；若流程要求完整核验，请在安装前检查该版本的资产和校验和。

如果 Codex Manager 能改善你的工作流，欢迎为仓库点 Star 以关注后续版本。

## 为什么需要 Codex Manager？

随着 Codex 使用变复杂，有价值的信息分散在 CLI、本地 rollout 文件、配置、账户状态和项目说明中。用户很难快速回答某个任务使用了什么模型和 reasoning effort、观察到了多少 token、哪些项目有 `AGENTS.md`，以及当前使用哪个账户或供应商。

Codex Manager 将这些视图集中到一个本地桌面控制中心，同时明确数据边界：数据标注来源，缺失值保持 `unavailable`，估算不会伪装成供应商账单。

## 核心功能

### 观测

- 浏览 session、turn、模型交互、模型、供应商、reasoning effort、耗时和 token 分类。
- 查看 input、output、cached read/write、reasoning output token 及来源/provenance。
- 活动筛选、游标分页和详情查看；不持久化消息正文或 reasoning 文本。
- 按版本化价格目录估算 API 等价成本；不是 ChatGPT 订阅扣费或供应商账单。
- 总览使用紧凑布局，最近活动默认只显示一条；首页 API 等价估算显示两位小数，底层覆盖口径不变。

### 管理

- 增量读取本地 `sessions` 与 `archived_sessions`，规范化到 SQLite。
- 发现已观测项目、Git root、worktree 和有效的 `AGENTS.md` 链。
- 只在明确授权根目录内创建、编辑、保存、恢复和查看 `AGENTS.md` revision。
- 通过官方 Codex login/App Server 边界查看账户、套餐、额度窗口和重置时间；macOS beta 支持多个 OAuth profile 及明确切换。
- 官方订阅首次成功读取后优先使用本地白名单缓存；用户可点击“刷新”强制重新读取，界面明确标注缓存时间和上次确认状态。
- 保存 Responses-compatible 自定义供应商，并在需要时显式启动本地网关。

### 本地优先

- 默认使用本机 SQLite；没有产品遥测或云同步。
- API Key、OAuth profile secret 和 gateway bearer 只进入应用专用 Keychain 或原生运行时内存。
- 可选 OTel receiver 和供应商网关在用户开启前均停止，并绑定本地接口。
- 不会静默改写 Codex 配置或后台换号。

## 快速开始

1. 从 [Releases](https://github.com/ewsun22/codex-manager/releases/latest) 下载最新 macOS `.dmg`。
2. 安装并启动 **Codex Manager**。
3. 首次启动时，应用会在配置的 Codex home（通常为 `~/.codex`）下发现本地数据。
4. 打开活动页面查看已观测的 session 和模型使用情况。
5. 编辑 `AGENTS.md` 前先在设置中授权项目根目录；使用供应商网关前先保存供应商，再明确启动 loopback listener。首页可查看并显式开启/关闭该 Responses 网关。

桌面应用不会自动修改 `config.toml` 或 `auth.json`。正式签名版本已完成 Developer ID signing、Apple notarization 和 stapling。

## 截图

所有截图均使用人工构造且已脱敏的演示数据。

| 活动与 token 用量 | 账户与额度 |
| --- | --- |
| ![活动与 token 用量](output/playwright/activity-turns-1280.png) | ![账户与额度](output/playwright/oauth-quota-desktop.png) |

| 项目与 `AGENTS.md` | 自定义 Responses 供应商 |
| --- | --- |
| ![项目与 AGENTS](output/playwright/final-desktop-project-agents.png) | ![自定义 Responses 供应商](output/playwright/codex-gateway-desktop.png) |

## 平台支持

- **已支持：** macOS 13+，当前发布资产为 arm64。
- **Windows：** unavailable；不宣称已有等价 secret store 和文件安全实现。
- **Linux：** unavailable；不宣称已有等价 secret store 和文件安全实现。

核心采集、规范化、存储、价格和项目解析保留跨平台接口，但不代表当前支持 Windows 或 Linux。

## 工作方式

- **Rollout adapter：** 增量读取本地 `sessions` 和 `archived_sessions` JSONL，只记录元数据。token 快照按 session、ordinal、事件类型和累计向量去重。
- **OTel adapter：** 可选的本地 TLS loopback、认证 OTLP/HTTP，可提供请求状态、耗时和有限分类；不会猜测性拼接 metrics 与 session。
- **CLI Schema 兼容性：** 通过短生命周期探测确认本机 CLI 是否支持 App Server Schema；这是非采集源，不附着 Codex Desktop 私有 stdio，也不影响 rollout 主采集。最近探测结果会持久化。
- **官方 App Server adapter：** 以独立短生命周期读取账户、套餐和 ChatGPT Codex 额度；不附着 Codex Desktop 私有 stdio，也不读取历史数据。
- **Filesystem 与 AGENTS adapter：** 发现已观测 cwd 和授权根目录，写入使用 canonical path、no-follow、冲突检查和原子替换保护。

## 可观测性与数据边界

`unavailable` 表示来源没有提供可验证字段，不是零值；`unobserved` 表示没有可靠终态，不等于成功、失败或仍在运行。Rollout JSONL 是内部兼容来源，不是稳定公开 API。OTel `codex.api_request` 可增加状态、耗时和固定分类，但不保存原始 host、path、query 或 body。当前 telemetry 通常不能可靠提供 wire response bytes 或请求级 TTFB，因此不宣称覆盖所有请求、严格实时完整性或完整供应商账单。

API 等价价格使用已识别模型调用和版本化本地价格目录；未知模型和字段不完整部分保持未覆盖。它不是 ChatGPT 套餐扣费、OpenAI Platform 账单或第三方供应商账单。详见[能力矩阵](docs/capability-matrix.md)和[架构说明](docs/architecture.md)。

## 账户、供应商与本地网关

官方订阅继续使用可信的 `codex login`、App Server、官方 credential store 和明确账户切换；OAuth access/refresh token 不会进入网关。首页提供现有 Codex Responses loopback 网关的快捷状态与显式“开启/关闭”按钮；网关默认停止，只绑定 `127.0.0.1`，支持 `/v1/models`、`/v1/responses` 和 `/v1/responses/compact`，不转换 Claude、Gemini 或 Chat 协议。首页不展示 bearer/API Key。远程上游必须是 HTTPS 公网 origin；拒绝 redirect、私网/link-local、userinfo、query 和 fragment。客户端 bearer 不转发，请求/响应 body、header、query 和完整上游错误不持久化。配置片段需原生确认，应用不会自动编辑 `config.toml`；停止前请恢复直连配置。

官方订阅页面采用 cache-first：没有缓存时首次读取成功后，在 macOS 独立 Keychain 中保存包含 email 的白名单账户摘要、套餐、额度窗口和时间；后续进入页面先读取本地缓存，只有用户点击“刷新”才强制实时读取。快照超过 6 小时、刷新失败或尚未观测时必须明确显示“陈旧/上次确认/unavailable”，不以旧数据伪装实时状态；即使可信 CLI 暂时缺失，已有安全快照仍可离线显示。不保存 token、原始 JSON、授权 URL 或错误正文。

Codex 配置页面聚焦供应商、网关与配置预览，已移除未开放的“全局提示词”“插件与市场”“会话管理”占位模块。侧栏的“本地桌面模式”同时显示应用版本和构建时间（可选短 SHA）；构建时间来自构建元数据，不代表启动时间。

导入档案的切换必须由用户明确触发，且仅在 Codex 最终解析的 `cli_auth_credentials_store` 为 `file` 时支持；`keyring` 和 `auto` 模式会 fail closed。CLI 与 IDE 扩展共享活动凭据文件，切换前应先结束正在运行的 Codex 任务。

## 安全与隐私

- 默认本地优先：没有产品遥测、云同步或自动上传。
- 不持久化消息正文、prompt、reasoning 文本、tool 参数/结果、`Authorization`、Cookie、OAuth code、API Key、gateway bearer、完整环境变量、请求 query 或原始 OAuth/App Server 响应。
- OAuth profile secret 在受限原生流程验证并存入应用专用 macOS Keychain；SQLite/WebView DTO 只暴露非秘密元数据。
- OTel 和网关为可选、认证、有限边界的本地 listener；网关在读取 body 前校验 host/origin/bearer，使用 header allowlist、上限并禁用 redirect。
- AGENTS 写入要求授权根目录、canonical path、符号链接和外部修改检查及原子替换。
- Release CI 包含固定 action SHA、secret scan、SBOM/许可证、资产哈希和受保护签名流程。

详见 [SECURITY.md](SECURITY.md)、[PRIVACY.md](PRIVACY.md)、[供应链说明](docs/supply-chain.md)、[能力矩阵](docs/capability-matrix.md) 和 [架构说明](docs/architecture.md)。

## 发布状态

稳定通道指向最新的非 draft、非 prerelease macOS arm64 Release。源码版本、tag 或草稿存在都不等于已公开；每个版本必须完成 exact-SHA CI、受保护签名、草稿复验、人工 Publish 和 published-mode 复验。

- `implemented`：供应商管理、本地网关、官方订阅视图、更新器和文档已进入 tag 源码。
- `tested`：Release/复验 workflow、自动化测试、构建、签名、公证、stapling 和公开资产检查按发布证据通过。
- `published`：只有非 draft、非 prerelease 且完成 published-mode 复验的 GitHub Release 才属于稳定通道。
- `observed`：本地 mock/loopback、桌面/浏览器 demo 和公开 Release 复验已有证据。
- `accepted`：真实 API Key 供应商 E2E、费用确认、手动配置往返/恢复，以及从可信旧签名应用升级安装仍是独立验收工作。
- `cleanup`：Release job 会在上传前删除临时签名材料；本地测试凭据/配置仍需操作者清理。

各版本的 source SHA、workflow、资产和验收限制见[发布运行手册](docs/release.md)。版本化 note 保留 tag 时的门禁快照，后续公开复验证据单独记录。早期 `v0.5.0-beta.1` 仍是 unsigned community prerelease，不是稳定下载。

## 开发

要求：Node.js 24+、Rust 1.98.0，桌面目标需要 macOS 13+。

```bash
npm ci
npm run typecheck
npm test
cargo test --workspace
npm run tauri:dev
```

详见 [docs/development-plan.md](docs/development-plan.md) 和 [docs/validation-2026-08-27.md](docs/validation-2026-08-27.md)。浏览器 demo 使用人工构造的脱敏数据，不会自动联网。

## 文档

- [能力矩阵](docs/capability-matrix.md) — 来源、字段、provenance 和不支持的宣称
- [架构说明](docs/architecture.md) — adapter、存储、去重和状态边界
- [开发计划](docs/development-plan.md) — 当前范围和后续工作
- [发布运行手册](docs/release.md) — 签名发布、复验、回滚和 cleanup 边界
- [安全策略](SECURITY.md) 与 [隐私说明](PRIVACY.md)

## 许可证

Codex Manager 使用 [Apache License 2.0](LICENSE) 发布。依赖和参考说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 免责声明

Codex Manager 是非官方社区项目。**与 OpenAI 无隶属或背书关系。** OpenAI、Codex 及相关标志归其各自所有者所有。请自行承担账户、供应商、配置和本地文件功能的使用责任，并遵守所连接服务的条款和隐私政策。
