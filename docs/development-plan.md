# 开发计划与状态

状态日期：2026-08-27。`implemented`、`verified locally`、`packaged`、`signed`、`notarized` 和 `released` 是不同边界。

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

## Phase 4：macOS beta 工程 — implemented, packaged and verified locally; external release gates pending

- [x] 隐私、路径安全、CSP 与显式 Tauri command capability
- [x] 原创应用图标与 macOS 13+ bundle 配置
- [x] Apache-2.0 LICENSE、NOTICE、第三方说明、隐私与安全文档
- [x] 固定 SHA 的 GitHub Actions、依赖审计、secret scan、SBOM 与许可证脚本
- [x] unsigned `.app`/`.dmg` 构建与 SHA-256 manifest 流程
- [x] 2026-08-27 本机构建 `Codex Manager 0.1.0` arm64 `.app`/`.dmg`，`hdiutil verify`、SBOM/许可证 JSON 和双层 SHA-256 manifest 验证通过
- [x] Playwright 验证总览、活动筛选/详情、项目/AGENTS 编辑 revision、OTel 凭据显式揭示与 390×844 布局
- [x] 完整安全扫描的 12 项发现已修复，并经独立只读验证
- [x] 应用内手动检查/确认安装更新，GitHub Releases 草稿工作流、Tauri 更新签名公钥与 `release` Environment 边界
- [x] 签名 release workflow 拆分为 secret-free build、受保护 sign、无签名 secret 的 draft，并加入 tag 不可覆盖、双 notary Accepted、实际 updater 验签、全资产回下载、attestation 与清理后置验证
- [x] 2026-08-29 Actions run `33225969842` 在 exact source `74bcb1d07449a817c850be3fd37a87582cd636f7` 完成 Developer ID Application/Hardened Runtime 签名；本机仍无可用 identity
- [x] 同一 run 的 app/DMG notarization 均为 `Accepted`，stapling 与 Gatekeeper 验证通过；该状态不得外推到后续 source SHA
- [x] `v0.2.0` unsigned community prerelease 已发布，且保持无 updater manifest 的独立通道
- [ ] `v0.2.1` 修复 SHA 的签名 beta 草稿、attestation、远端下载验证与人工发布（Environment 已具备；需重新运行并人工审批）

## 后续阶段

### Windows beta

- Windows reparse point / symlink containment 和原子替换等价实现。
- Windows Codex executable/home 发现、WebView2、MSIX/installer 与签名。
- Windows 文件 watcher、长路径、大小写与多 worktree 验收。

### 可选精确网络观测

只有产品明确需要完整 URL、真实 TTFB、wire bytes 或中转站路由时，才单独设计可选反代。该模块必须默认关闭、独立授权、避免记录 body/credentials，并另做证书、代理配置和流式协议安全评审。
