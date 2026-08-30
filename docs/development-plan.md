# 开发计划与状态

状态日期：2026-08-30。`implemented`、`verified locally`、`packaged`、`signed`、`notarized`、`released` 和 `updater E2E accepted` 是不同边界。

## Phase 0：可行性探针 — completed

- [x] 仓库、隐私和架构基线
- [x] 完整 JSONL 行增量读取与脱敏 fixture
- [x] session/turn/item/model-call 规范化
- [x] token 重复快照与非单调回退处理
- [x] 版本化价格规则与严格类型检查
- [x] Desktop 与 PATH Codex CLI 基础兼容验证

## Phase 1：本地数据层 — implemented and verified locally

- [x] Rust 1.98 Cargo workspace 与 Tauri 2 壳
- [x] Rust rollout adapter 与 metadata-only model
- [x] SQLite migrations、WAL、checkpoint、resume state 和游标分页
- [x] watcher 去抖、合并、单 worker 与周期 reconciliation
- [x] loopback OTLP/HTTP logs/metrics receiver、body limit 与 allowlist
- [x] App Server Schema hash/capability probe、受限子进程与临时产物上限
- [x] retention、source health、错误脱敏和不完整行恢复

## Phase 2：使用记录 UI — implemented and verified locally

- [x] 总览、活动表格、筛选、游标分页与详情抽屉
- [x] 模型、effort、provider、token、缓存、耗时、状态、endpoint 与价格展示
- [x] cache-write、reasoning output、source/confidence 和 `unavailable`
- [x] 价格目录与来源说明
- [x] 采集源健康、OTel 开关和 Codex capability 页面
- [x] 桌面/移动窗口响应式与浏览器 demo adapter

## Phase 3：项目与 AGENTS — implemented and verified locally

- [x] 已观测 cwd、Git root、worktree 与用户授权根目录发现
- [x] 全仓 AGENTS 候选发现与全局到 selected cwd 的 effective chain
- [x] `AGENTS.override.md` / `AGENTS.md` / fallback precedence 与最大字节规则
- [x] 项目根 `AGENTS.md` 显式创建
- [x] canonical path、symlink/no-follow、文件名与授权根校验
- [x] SHA/mtime 冲突、同目录原子替换、fsync、两阶段 revision 与崩溃恢复
- [x] 每文件最多 20 个本地 revision，支持带当前 SHA 的恢复

## Phase 4：macOS beta 工程 — signed stable channel active; updater E2E pending

- [x] 隐私、路径安全、CSP 与显式 Tauri command capability
- [x] 原创应用图标与 macOS 13+ bundle 配置
- [x] Apache-2.0 LICENSE、NOTICE、第三方说明、隐私与安全文档
- [x] 固定 SHA 的 GitHub Actions、依赖审计、secret scan、SBOM 与许可证脚本
- [x] unsigned `.app`/`.dmg` 构建与 SHA-256 manifest 流程
- [x] 2026-08-27 本机构建 `Codex Manager 0.1.0` arm64 `.app`/`.dmg`，`hdiutil verify`、SBOM/许可证 JSON 和双层 SHA-256 manifest 验证通过
- [x] Playwright 验证总览、活动筛选/详情、项目/AGENTS 编辑 revision、OTel 凭据显式揭示与 390×844 布局
- [x] 完整安全扫描的 12 项发现已修复，并经独立只读验证
- [x] 应用内手动检查/确认安装更新，GitHub Releases 草稿工作流、Tauri 更新签名公钥与 `release` Environment 边界
- [x] 桌面端启动后按需及定时更新检查：默认间隔 12 小时，可配置 1–168 小时；受限元数据持久化、失败保留上次成功状态，以及设置侧栏/应用更新卡片提醒
- [x] Playwright 验证桌面宽屏与 390×844 窄屏的设置侧栏/更新卡片提醒；浏览器 demo 未自动联网
- [ ] 已安装桌面版的真实周期触发，以及从旧签名版本到新签名版本的更新安装/重启验收
- [x] 签名 release workflow 拆分为 secret-free build、受保护 sign、无签名 secret 的 draft，并加入 tag 不可覆盖、双 notary Accepted、实际 updater 验签、全资产回下载、attestation 与清理后置验证
- [x] release eventual-consistency 状态机、稳定 tag updater URL、fail-closed Node secret scanner、provenance v2、exact draft resume 和只读 post-publish verifier 已随 `606a927c4f536b0f15a22a8ec850b6c99a97a117` 推送并由 Actions run `33239136597` 实测 preflight/build/sign 门禁
- [x] 2026-08-29 Actions run `33225969842` 在 exact source `74bcb1d07449a817c850be3fd37a87582cd636f7` 完成 Developer ID Application/Hardened Runtime 签名；本机仍无可用 identity
- [x] 同一 run 的 app/DMG notarization 均为 `Accepted`，stapling 与 Gatekeeper 验证通过；该状态不得外推到后续 source SHA
- [x] `v0.2.0` unsigned community prerelease 已发布，且保持无 updater manifest 的独立通道
- [x] 精确复核并删除旧 No-Go draft `378845893`；只对 `606a927c4f536b0f15a22a8ec850b6c99a97a117` dispatch 一次并完成人工 sign 审批，app/DMG sign/notary/staple/Gatekeeper、本地全链和 credential cleanup 通过
- [x] `v0.2.1` exact source `fbe42de828f4f64549d9a4b36bc506fd15b11aa2` 完成 draft 全链、人工 Publish 和匿名 post-publish 回下载复验；release run `33240800663`、published verify run `33241992763` 均成功
- [x] 本机 `/Applications/Codex Manager.app` 的 `v0.2.1` 通过 strict codesign、Notarized Developer ID Gatekeeper 与 stapler 验证，可作为可信更新起点
- [x] `v0.2.2` exact source `a2359e20eda9423f509a6a76f59d3e4c61a10ddc` 完成签名发布与公开复验；release run `33245641061`、published verify run `33246307716` 均成功，九项资产和 Release ID `378939150` 已核对
- [ ] 尚未取得从已安装 `v0.2.1` 到 `v0.2.2` 的真实 updater E2E 证据；不得把签名发布与公开复验外推成客户端更新验收
- [x] `v0.3.0` 已从 exact source `fab415af2d9da6dfa41e79c7073bfb9fbb5fb818` 完成签名发布与公开复验；release run `33254920672`、published verification run `33255604307` 均成功，Release ID 为 `378997476`，九项资产已核对
- [x] `v0.4.0` 已从 exact source `e462db05aa1f2608509cbf333d992bbd89be9a1d` 完成签名发布与公开复验；release run `33278181464`、published verification run `33278878143` 均成功
- [ ] 尚未取得从已安装旧签名版本到 `v0.4.0` 的真实检查、下载、验签、安装、重启 updater E2E 证据
- [x] `v0.5.0` 已从 exact source `fdc96c38de8d6d6cacc46a9b7fc70157d39439ee` 完成签名发布与公开复验；CI run `33284042848`、release run `33284510727`、published verification run `33285144491` 均成功，Release ID `379147257` 与九项资产已核对
- [ ] 尚未取得从已安装的可信旧签名版本到 `v0.5.0` 的真实检查、下载、验签、安装、重启 updater E2E 证据
- [x] `v0.5.1` 已从 exact source `b933401353de46e39b1ea8e3f6ef1fb8065552b9` 完成签名发布与公开复验；CI run `33293059226`、release run `33293480453`、published verification run `33294184588` 均成功，Release ID `379187149` 与九项资产已核对
- [ ] 尚未取得从已安装的可信旧签名版本到 `v0.5.1` 的真实检查、下载、验签、安装、重启 updater E2E 证据

## 后续阶段

### Phase 5：Codex 供应商与 Responses 网关 — stable channel active; real-provider acceptance pending

- [x] 供应商非秘密元数据使用统一 SQLite store；API Key 使用应用专用 macOS Keychain，WebView DTO 只返回 `hasApiKey`
- [x] 新增表格优先的“供应商与反代”界面，并把既有 OAuth/认证档案/额度入口明确命名为“官方订阅”
- [x] `v0.5.1` 将供应商工作流重组为“Codex 配置管理”卡片与弹窗；保留 Keychain-only、显式启动和原生确认边界，模型测试只给出不发送请求的说明，通用配置或自动接管开关保持禁用
- [x] 显式启动/停止固定 IPv4 loopback 的 Responses identity gateway；随机本机 bearer、body/并发限制、受限响应 header 与 redirect disabled
- [x] 只支持 `/v1/responses` 与 `/v1/responses/compact`；不把官方 OAuth token 放入反代池
- [x] 远程 HTTPS 公网和显式 loopback HTTP 两类上游；拒绝 userinfo、query、fragment、私网/link-local 等地址
- [x] 原生确认后只显示手动配置片段；本版不自动改写 `config.toml` 或 `auth.json`
- [x] `v0.5.0-beta.1` 已从 exact source `dd370070ae58c3e70d61e95f3ad7b38278080a50`、CI run `33282176058` 发布为 `Unsigned Community Build` GitHub Pre-release；Release ID `379137424`，四项公开资产已回下载复验且稳定 updater 仍为 `v0.4.0`
- [x] `v0.5.0` 版本、正式 release note、适用文档和 main-only CI push 触发边界已定版，并从最终 exact source `fdc96c38de8d6d6cacc46a9b7fc70157d39439ee` 完成单次主线 CI、受保护签名/双公证、九资产草稿复验、人工 Publish 与 published-mode 公开复验
- [x] `v0.5.1` 配置页紧凑卡片/弹窗、响应式布局、双语开源展示与稳定下载入口已发布，并完成签名、公证、stapling、草稿与公开后复验
- [ ] 用用户授权的真实 API-key 上游完成 Codex CLI Responses E2E、费用确认与恢复直连验收
- [ ] 从已安装的可信签名旧版本真实检查、下载、验签、安装、重启到最新稳定版本，完成 updater E2E 验收
- [ ] 为运行中配置漂移、退出前恢复、崩溃恢复设计受管 config state machine；在此之前网关不会自动接管 Codex

首版明确不做 Chat/Anthropic/Gemini 转换、provider failover/retry pool、远程 provider catalog、自定义 header DSL、请求正文日志、官方订阅代理或后台自动换号。

### Windows beta

- Windows reparse point / symlink containment 和原子替换等价实现。
- Windows Codex executable/home 发现、WebView2、MSIX/installer 与签名。
- Windows 文件 watcher、长路径、大小写与多 worktree 验收。

### 可选精确网络观测

Phase 5 的网关只完成默认关闭的 Codex Responses 透明转发和进程内状态，不把完整 URL、TTFB、wire bytes 或上游账单写入活动库。若后续需要精确网络观测，必须另行设计字段 provenance、留存、流式终态、配置接管/恢复和真实 Codex E2E，不得从“listener 已启动”推导为业务验收。
