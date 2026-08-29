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

## Phase 4：macOS beta 工程 — signed stable channel released; v0.4.0 release pending

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
- [ ] `v0.4.0` 从最终 `main` exact SHA 完成 fresh signed release、公开复验，以及从已安装旧签名版本下载、验签、安装、重启的 updater E2E

## 后续阶段

### Windows beta

- Windows reparse point / symlink containment 和原子替换等价实现。
- Windows Codex executable/home 发现、WebView2、MSIX/installer 与签名。
- Windows 文件 watcher、长路径、大小写与多 worktree 验收。

### 可选精确网络观测

只有产品明确需要完整 URL、真实 TTFB、wire bytes 或中转站路由时，才单独设计可选反代。该模块必须默认关闭、独立授权、避免记录 body/credentials，并另做证书、代理配置和流式协议安全评审。
