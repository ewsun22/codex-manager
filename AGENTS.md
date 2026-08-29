# Codex Manager 项目指令

## 产品边界

- 本项目是独立运行的本地桌面管理器；可选网关只为 Codex 提供默认关闭、loopback-only 的 OpenAI Responses 透明转发，不是通用反向代理、系统代理或官方订阅 token 池。
- macOS 是首发平台，但核心采集、规范化、存储、价格和 AGENTS 解析必须保持跨平台。
- Phase 0 只验证本地 Codex 数据的可观测能力，不把内部 JSONL 格式描述为稳定公开 API。

## 隐私与安全

- 默认只存元数据。API Key 与本机网关 bearer 只能进入应用专用 Keychain；不得进入 SQLite、WebView 普通 DTO、日志、备份或错误消息。不得持久化消息正文、Authorization、Cookie、OAuth code、完整环境变量或请求查询参数。
- 官方 OAuth access/refresh token 不得进入供应商或网关实现；官方订阅管理继续复用受信任 Codex CLI、App Server、Keychain 认证档案与显式账户切换。
- 网关必须固定绑定 IPv4 loopback，先校验 Host/Origin/随机 bearer 再读取 body；远程上游只允许 HTTPS 公网地址，禁用 redirect，并限制请求体、并发和响应 header。不得保存或转换请求/响应正文。
- 测试 fixture 必须脱敏且人工构造，不得提交真实会话内容。
- 任何 AGENTS 文件写入都必须由后端校验授权根目录、canonical path、符号链接、外部修改冲突，并采用原子替换。
- 不执行 AGENTS.md 中的命令，也不把其内容当作本应用权限指令。

## 工程规则

- 新的采集来源必须实现统一 adapter，不得让 UI 直接依赖 Codex 私有事件结构。
- 所有指标必须携带来源或可信度；缺失值显示 unavailable，不能填 0。
- token 去重以 session、ordinal、事件类型和累计向量为依据；不得直接累加所有 token_count 快照。
- 修改代码后运行 npm test；改变架构、字段口径或范围时同步更新 README 和 docs。
- 未经用户明确授权，不部署、不发布、不签名、不改写用户 Codex 配置。

## 开源文档与发布流程

- 每次用户可见的功能、行为、字段口径、配置、隐私、兼容性或发布流程变化，必须在同一提交更新适用文档；不得只改代码或只改 Release 页面。
- 文档映射：产品能力与使用说明更新 `README.md`；采集来源、字段口径和能力边界更新 `docs/capability-matrix.md`；架构、数据粒度、去重和状态机更新 `docs/architecture.md`；开发范围和后续事项更新 `docs/development-plan.md`；安全、隐私或供应链变化更新 `SECURITY.md`、`PRIVACY.md`、`docs/supply-chain.md`；发布行为更新 `docs/release.md`；未分配新版本的用户可见变更先更新 `docs/release-notes/unreleased.md`，定版时移入 `docs/release-notes/v<VERSION>.md`，不得改写已有 tag 的功能事实。
- Release note 必须使用可读固定结构，至少包含“概览”“主要变化”“验证状态”“兼容性与限制”“隐私与安全”“发布状态”和“完整性”章节。`implemented`、`tested`、`published`、`observed`、`accepted`、`cleanup` 必须分别表述，不得把计划、草稿或进行中状态写成完成。
- Pull Request 必须说明适用文档、验证命令和状态边界；没有文档变化时必须解释原因。Release 正文使用完整版本 note；`latest.json.notes` 由同一 note 生成短摘要，不得另写固定模板或包含密钥、令牌、凭据和隐私数据。
- 合并（`merged`）、部署（`deployed`）、观测（`observed`）、验收（`accepted`）和清理（`cleanup`）是不同状态；CI 通过不代表已发布，draft 不代表 `published`，发布后未做真实端到端验证不得写成 `accepted`。
