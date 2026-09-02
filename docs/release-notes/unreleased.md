# Codex Manager 未发布变更

> `v0.6.5` 定版后，新的用户可见变更继续记录在这里；不得改写已有 tag 的版本事实。

## 概览

本地代理停止后立即重新读取 OAuth 档案，让界面同步显示后端 checkpoint 的最新时间。

## 主要变化

- `implemented`：停止 CLIProxyAPI 反代成功后复用既有刷新链，重新读取网关状态与 OAuth 档案，避免“上次 checkpoint”停留在停止前的时间。
- `implemented`：测试入口改用 Node.js 24 原生自动发现，并对全部 WebView command 与 Tauri allow 权限做精确集合校验。

## 验证状态

- `tested`：已增加停止成功、保留已停止状态、再刷新 checkpoint 状态的顺序回归断言；并将全部 Node.js 测试、TypeScript 类型检查与文档同步校验作为本切片门禁。源码链确认原生停止会 checkpoint 运行期 OAuth 档案，前端随后重新读取脱敏档案 DTO。
- `observed`：暂无新增运行时观测证据；不得将静态与自动化证据外推为真实 OAuth 业务验收。

## 兼容性与限制

无新的平台、配置格式或 API 兼容性变化；停止行为和 checkpoint 持久化语义不变。

## 隐私与安全

无新的凭据流向或权限变化；前端仍只重新读取脱敏档案 DTO，API Key、token、bearer 和消息正文仍不得进入普通 DTO、SQLite 或日志。

## 发布状态

- `published`：否；本切片尚未分配新版本，未创建 tag 或 Release。
- `accepted`：否；尚未使用用户授权的真实 CLIProxyAPI OAuth 凭据完成启动、停止、checkpoint 时间刷新与 Codex Responses 业务链验收。
- `cleanup`：本切片不创建新的运行资源或临时发布状态。

## 完整性

本文档记录未发布状态；没有新版本号、tag、exact source SHA 或发布资产。
