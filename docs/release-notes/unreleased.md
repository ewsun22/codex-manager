# Codex Manager 未发布变更

> `v0.6.2` 定版后，新的用户可见变更继续记录在这里；不得改写已有 tag 的版本事实。

## 概览

新增中英文界面和设备语言自动判断。

## 主要变化

- `implemented`：新增源码级简体中文/English 目录与切换器、设备语言默认判断、本地偏好保存和 locale-aware 数字/日期/金额格式化；不会扫描或改写已渲染 DOM，动态项目数据保持来源原文。

## 验证状态

- `tested`：`npm run typecheck`、100 项 `npm test`、`npm run build`、`npm run docs:check` 与 `npm run tauri:build` 均通过；新增测试覆盖中文设备、非中文设备、手动覆盖、恢复自动、locale 格式化、动态值保留和主要界面英文目录完整性。
- `observed`：浏览器 demo 已逐页观察 8 个主页面和官方订阅 3 个子页，并实际完成 English、中文及恢复设备语言切换；本地 release `.app` 与 `.dmg` 已生成，release 进程和 WebKit 子进程成功启动。由于同一 bundle identifier 同时存在安装版、debug 版和 release 版，本机辅助功能无法取得唯一桌面窗口，因此没有把桌面截图写成已观察证据。

## 兼容性与限制

不改变 Rust/SQLite 设置契约。首次启动按 `zh-*` 中文、其他语言 English；项目名、模型名、路径、外部 Release notes 和底层技术错误保留原文。

## 隐私与安全

语言偏好只进入 WebView 本地存储，不进入普通 DTO、SQLite、日志或 Keychain。API Key、token、bearer 和消息正文边界不变。

## 发布状态

- `published`：否；本次没有提交、推送、tag、Release、签名或部署。
- `accepted`：本地源码实现、自动化回归、浏览器业务界面和未签名 Tauri 构建已验收；已安装客户端更新链、签名、公证和发布后观测不属于本切片。
- `cleanup`：验收用 Vite 服务和本地 release 应用进程在交付前停止；构建产物保留在 `target/release/bundle` 供本地复核。

## 完整性

变更仍为未发布工作树状态，没有新版本号、tag、exact source SHA 或远端发布资产；本地构建产物不等于已发布资产。
