# Codex Manager 未发布变更

> `v0.5.0` 定版后，新的用户可见变更继续记录在这里；不得改写已有 tag 的版本事实。

## 概览

供应商工作流已重组为更紧凑的“Codex 配置管理”页面；本次只改变信息架构和交互呈现，不扩大配置文件、Keychain 或 OAuth 权限边界。

## 主要变化

- 供应商由表格与常驻表单改为紧凑卡片和新增/编辑弹窗，模型、reasoning effort、Base URL、Keychain 状态与主要操作集中在同一卡片。
- 桌面供应商列表使用固定五列栅格统一名称、模型、凭据、状态与操作对齐；窄屏改为带字段标签的两列信息区和统一按钮栅格。
- 页面就近展示网关状态、配置预览入口、官方订阅边界，以及全局提示词、插件与会话管理的后续入口。
- “应用”明确表示启动所选供应商的本机 loopback 网关；未实现的模型测试命名为“测试说明”并明确不会发送请求，通用配置和自动接管开关保持禁用并说明原因。

## 验证状态

- `implemented`：Codex 配置管理页面、供应商弹窗与安全状态说明已实现；未新增后端写配置命令。
- `tested`：`npm test` 89/89、`npm run build` 与 `npm run docs:check` 通过；本地浏览器完成桌面五列对齐、390 × 844 响应式与无水平溢出、供应商 modal、菜单和弹窗 Escape 关闭、非敏感预览、应用状态及官方订阅跳转验收，干净页面控制台 error 为 0。

## 兼容性与限制

- 仅支持现有 OpenAI Responses-compatible 供应商；没有新增 Chat/Anthropic/Gemini 转换。
- 页面仍不会自动读写 `config.toml` 或 `auth.json`；完整配置片段仍需要原生确认后临时显示。
- 官方订阅仍由独立页面和受信任 Codex CLI/App Server/Keychain 链路管理，不会进入第三方网关。

## 隐私与安全

没有新增秘密存储或 WebView 权限。供应商 API Key 继续是 write-only input，只进入应用专用 Keychain；普通卡片只显示 `hasApiKey` 状态，不显示掩码密钥。OAuth token、`auth.json` 和本机 bearer 的现有隔离与原生确认边界保持不变。

## 发布状态

- `published`：否；本文件不对应 tag 或 Release。
- `observed`：已在 `127.0.0.1` 脱敏 demo 中观测新页面和主交互；未在真实桌面应用中保存 provider、启动网关或读取 Keychain。
- `accepted`：否；真实供应商网络请求、配置接管和发布安装均不在本切片验收范围。
- `cleanup`：无运行时资源；视觉验收截图仅保存在忽略目录。

## 完整性

没有未分配版本号、tag、exact source SHA 或发布资产；本文件不改变 `v0.5.0` 已有 tag/release 事实。
