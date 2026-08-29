# Codex Manager 未发布变更

> 本文档收集 `v0.2.2` 之后已实现但尚未分配新 SemVer 的用户可见变更。发布时必须移入对应 `v<VERSION>.md`，不得改写已有 tag 的版本事实。

## 概览

活动记录改为任务与模型交互分层展示，并同步修正 canonical 去重、耗时、价格覆盖、项目更新时间与开源文档流程。

## 主要变化

- 活动页默认以 `turn` 任务汇总展示终态和任务级耗时；单次 token 事件放在“模型交互”视图，已记录但不伪造调用级耗时。
- 已归档或已出现后续任务但缺少终态的记录显示“未观测到结束”；OTel request 保留真实 HTTP 状态和请求耗时。
- token 表格不再逐格重复“高可信”，来源与推导方式收纳到数据说明和详情；顶部、底部都可切换分页。
- 项目列表显示最后一次已观测对话时间，长路径可换行。
- API 等价估算按 canonical model call 的 token 逐次计价，未知模型保留为未覆盖，不使整卡失效。
- CI 校验 release note 章节和状态边界；GitHub Release 使用完整 note，`latest.json.notes` 从同一文档生成摘要。

## 验证状态

- `implemented`：canonical 指纹 v2、活动分层、部分 token 计价、项目时间、UI 与文档流程已写入当前工作树。
- `tested`：Node/Rust 单元测试、类型检查、构建、文档校验、历史 DB 副本迁移和 Playwright 演示模式验收已通过。

## 兼容性与限制

内部 schema 升至 v9。从 v8 首次启动时会一次性重算历史 logical fingerprint；原始采集行不会删除，后续启动只补空值。

## 隐私与安全

不新增网络、OAuth、WebView 或文件系统权限；继续只保存元数据，不保存消息正文和凭据。

## 发布状态

`published`：未发布。`observed`：仅观测到本地测试和演示模式结果。`accepted`：仅完成当前开发切片验收，不代表签名发布或 updater E2E。`cleanup`：无云资源、凭据或发布临时产物需清理。

## 完整性

尚未分配新 tag、exact source SHA 或发布资产。实际发布前必须将本文档转为对应版本 note，并按 `docs/release.md` 记录签名、公证、回下载、观测和清理证据。
