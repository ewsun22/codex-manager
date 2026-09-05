# Codex Manager 未发布变更

> `v0.6.5` 定版后，新的用户可见变更继续记录在这里；不得改写已有 tag 的版本事实。

## 概览

在既有停止后 OAuth checkpoint 刷新基础上，增加任务用量完整性、增量计价一致性、保留期维护、AGENTS 草稿保护、活动筛选、原生代理稳定性、变更文件优先扫描和可靠确认框改进。此为当前工作源码的未发布切片，版本仍为 `0.6.5`。

## 主要变化

- `implemented`：停止 CLIProxyAPI 反代成功后复用既有刷新链，重新读取网关状态与 OAuth 档案，避免“上次 checkpoint”停留在停止前的时间。
- `implemented`：测试入口改用 Node.js 24 原生自动发现，并对全部 WebView command 与 Tauri allow 权限做精确集合校验。
- `implemented`：异常快照触发后，任务总用量保持 `unavailable`；有效调用仍参与部分估价。活动与计价共用缺失模型/provider/effort 回退规则，读取批次边界不再改变缺失字段的计价输入。
- `implemented`：保留期截止时间在启动、设置变更和按小时维护时更新，并在每次导入事务中应用；同步清理过期终态任务、关联调用/timeline，保留 checkpoint 并更新删除水位。
- `implemented`：AGENTS 未保存保护覆盖文件、项目、主导航和窗口关闭；迟到读取不覆盖当前文件，保存期间锁定输入。原生字符串错误保留可操作信息，写入成功但刷新失败时分别反馈。
- `implemented`：活动模型和推理级别筛选候选覆盖所选视图全部记录；保留零结果时的选中值，切换视图清理不兼容的结果条件。
- `implemented`：首页与本地代理页复用可见时的状态校验，不周期读取 OAuth 档案。耗时原生操作移入阻塞工作池，启动健康检查增加 10 秒硬截止期、500ms 请求超时并禁止重定向；状态查询不清理 runtime 或恢复安装事务。

- `implemented`：watcher 变更文件优先、有界历史发现游标和公平续读队列，保留读取预算及每 60 秒 reconciliation；不再要求每次增量前遍历并排序全部历史文件。
- `implemented`：8 处 WebView 确认改用应用内对话框，覆盖 AGENTS、Codex 配置和代理 OAuth 档案移除；取消、重复请求或页面卸载不会执行待确认写入。
- `implemented`：文档检查增加双语版本/平台/下载事实、本地链接/锚点/非空资产和可选外部 URL 验证；提供默认关闭且不能进入 release 的隔离桌面 QA feature。

## 验证状态

- `tested`：Node 125 项、默认 Rust 148 项通过（4 项显式 ignored），类型检查、fixture probe、前端构建、Clippy、格式和差异检查通过。ignored 人工扫描基准与真实内核下载/空池测试分别执行通过。
- `observed`：1k/10k/50k 人工增量扫描、真实 Tauri 人工数据的 8 项检查与实际允许关闭，以及审核版 CLIProxyAPI 的 loopback、401/200/400/404 和停止清理已有证据。详见 [S5–S6 验证记录](../validation/reliability-s5-s6.md)，之前的浏览器结果保留于 [S1–S4 验证记录](../validation/reliability-s1-s4.md)。
- `observed`：未执行真实 OAuth 成功请求、上游 5xx 或费用验收；updater 安装按用户要求跳过，不写为通过。

## 兼容性与限制

无新平台或 Codex 配置格式变化；新增 SQLite 迁移 14 和受限活动候选读取命令。已有非空调用模型保持原值。未观测终态或缺少有效时间的任务暂不自动清理；OAuth 和 AGENTS revision 的独立保留策略不变。大目录基准采用人工数据，结果不代表所有真实目录；真实上游协议仍待指定 OAuth 测试档案、模型与费用边界。updater 本轮明确跳过。

## 隐私与安全

凭据信任域、内核审核基线和秘密保存规则不变。主窗口新增活动候选读取权限及用于未保存关闭保护的 `core:window:allow-destroy`；没有新增通用文件、HTTP、shell、process 或 dialog 插件权限。应用内 HTML dialog 不增加原生权限；调试 QA 使用人工数据、禁用认证 backend 和敏感命令，release 构建拒绝 QA feature。AGENTS 草稿只在内存保存。API Key、token、bearer 和消息正文仍不得进入普通 DTO、SQLite 或日志。

## 发布状态

- `published`：否；本切片尚未分配新版本，未创建 tag 或 Release。
- `accepted`：否；尚未使用用户授权的真实 CLIProxyAPI OAuth 凭据完成启动、停止、checkpoint 时间刷新与 Codex Responses 业务链验收。
- `cleanup`：本切片不创建云资源或临时发布状态；隔离桌面与内核测试均正常结束，人工数据库和脱敏证据保留在测试目录；浏览器验证已结束并关闭测试标签，开发服务器已停止。

## 完整性

本文档记录未发布状态；没有新版本号、tag、exact source SHA 或发布资产。
