# v0.6.7 发布准备与证据

状态日期：2026-09-06。用户授权：“提交，推送，发布 0.6.7，更新我自己来”。

## 变更及验证

本补丁将 SQLite 主数据库预算从默认页大小下的 512 MiB 调整为固定 1 GiB，按实际 `page_size` 计算上限，保留事务迁移及有界写入。原 v13 库在旧预算下迁移 14 会因索引增长触发 `SQLITE_FULL`，真实私有副本已通过修复后的生产 `Store::open`，已有 14 张业务表的行数、全部 ingest checkpoint 保持一致。原始数据库未替换，私有副本已清理。详见[故障诊断与回归记录](startup-migration-capacity.md)。

本地最终版本检查：`npm run check`（125 项 Node 测试、类型检查及人工 fixture probe）、`npm run build`、存储 19 项测试及最终 3 项容量回归、Rust 格式、secret scan 和 release dry-run 均通过；双语 README、版本 note 与验证文档通过 GitHub GFM 渲染检查。

## 用户可见面同步检查

| 用户可见面 | 本次处理 |
| --- | --- |
| 英文 README / 中文 README | 同步 0.6.7 容量修复、0.6.6 已知问题、证据入口及用户自行安装边界；保留准确标注版本的历史功能说明 |
| 应用版本 / 平台 | package、lock、workspace、Tauri 和 demo 同步 0.6.7；macOS 13+ Apple silicon 不变 |
| 截图 / GIF / UI 文案 | no change：没有布局、交互或业务文案变更，原 0.6.6 人工验证截图保留明确版本标签，不伪造 0.6.7 安装画面 |
| 架构 / 计划 / 验证文档 | 同步固定 1 GiB、实际页大小、WAL/临时文件不计入预算，以及原库副本迁移与数据保留证据 |
| 安全 / 隐私 | 明确用户本版发布授权、真实 OAuth 未验收、固定数据库预算及私有副本已清理；认证和数据字段不变 |
| 能力矩阵 / 供应链 / 许可证 | no change：不新增采集字段、网络监听、权限、依赖或内核版本；本版 SBOM/许可证由 exact source 重新生成 |
| Release / latest.json.notes | 使用同一 v0.6.7 note 生成完整正文和短摘要；不改写旧 tag note；unreleased 重置 |
| 下载 / Quick Start / 徽章 | no change：保留动态 latest 稳定下载入口，公开后重新核实指向 0.6.7 |
| GitHub Description / Topics | no change：产品能力、定位、支持平台未变化，发布前只读核对 |
| GitHub homepage / 网站 / 商店 | no change：homepage 为空，没有网站或商店上线操作 |
| 回退 / 清理 | 保留已有 tag 和资产；以后回退也必须使用更高 SemVer。保留用户原有 `design-qa.md`；本机更新由用户自行执行 |

发布前检查版本、链接、非空图片、双语事实、Markdown 渲染、secret 和差异；最终 exact SHA 通过 CI、无秘密 build、受保护签名、app/DMG 公证、stapling、Gatekeeper、updater 验签、SHA-256、SBOM/许可证、provenance/attestation 和草稿资产复验后才 Publish。随后独立执行只读 published 复验并匿名核验 latest。

## 状态与边界

`implemented` / 本地 `tested` 已完成；源码定版不提前宣称 `published`。公开验证状态在执行完成后另记，不改写不可变 tag。用户本机安装和实际启动仍待用户操作，不记为 `accepted`；真实 OAuth 业务验收仍未完成。本版限定发布授权见 [SECURITY.md](../../SECURITY.md)。

`deployment_decision=project-authorized`：用户明确指定 v0.6.7 的提交、推送与发布；不包含代理替换本机应用、认证切换或付费业务调用。
