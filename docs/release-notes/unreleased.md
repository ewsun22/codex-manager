# Codex Manager 未发布变更

> `v0.6.0` 定版后，新的用户可见变更继续记录在这里；不得改写已有 tag 的版本事实。

## 概览

本切片把反代职责交给独立版本的 CLIProxyAPI 官方预编译 sidecar，并将产品拆为“Codex 配置”和“本地代理”两个控制面。Codex Manager 负责配置档案、显式下载/校验/安装/启停与脱敏状态，本机不编译 Go 内核。

## 主要变化

- `implemented`：桌面主体与 CLIProxyAPI 内核独立版本；用户可检查、安装和更新官方 GitHub Release，首页与反代页显示当前/最新版本、PID 和更新状态。
- `implemented`：按 OS/arch 和稳定 tag 选择精确预编译资产，强制 GitHub Release asset digest 与 `checksums.txt` 双 SHA-256 一致；下载/解压有界，经私有 staging 安装，安装切换与健康失败回滚的持久事务标记可恢复中途强制终止；上一版保留到新内核通过 `/healthz`，失败自动回滚。
- `implemented`：首页提供显式开关和 OpenAI、Claude、Gemini 兼容 API URL；状态 DTO 不展示 bearer、API Key 或原始配置，请求计数在关闭 CLIProxyAPI usage stats 后显示 `unavailable`。
- `implemented`：生成配置强制 `127.0.0.1`、非空 client key、`commercial-mode:true`，关闭 remote management、control panel、plugins、request/debug/file logs、usage statistics 与 retry；受管 child 以 PID + `/healthz` 判定并只停止自己的进程。
- `implemented`：供应商 API Key 静态保留在 Keychain，启动时物化到应用私有 `0700` runtime 目录中的 `0600` 配置，受管进程停止时删除；不进入 SQLite、普通 WebView DTO、日志或备份。
- `implemented`：CLIProxyAPI OAuth 档案经原生文件选择器显式导入，独立存入 `cc.codex.manager.cliproxy-auth.v1`；支持 `codex`/`claude`/`antigravity`/`kimi`/`xai` 当前理解格式，DTO 不含账户、路径、token、fingerprint；运行时投影与停止前 provider/identity/CAS checkpoint 已接入，pending 证据 fail closed。
- `implemented`：Codex 配置页面支持官方直连、本地 CLIProxyAPI、外部 Responses provider 的预览、CAS/原子事务应用与恢复；采用 `model_providers.<id>.auth.command`，不再生成旧 bearer/reveal 片段。
- `implemented`：三信任域隔离，CLIProxyAPI Management API/OAuth callback 不向 WebView 开放；官方订阅保持独立 cache-first 链路。

## 验证状态

- `tested`：`cargo check -p codex-manager-desktop`、严格 `cargo clippy -p codex-manager-desktop --all-targets -- -D warnings`、桌面端 108 项 Rust 测试（105 通过，3 项需已安装签名 ChatGPT bundle 或实时下载官方内核而跳过）、存储层 9/9、`npm run typecheck`、前端 95/95、`npm run build`、Rust 格式和 `git diff --check` 均通过。Playwright 演示模式已验证首页紧凑开关、两个控制面、配置预览/应用交互、OAuth 凭据池启停与 390×844 窄屏布局；演示模式不等于原生 sidecar 与真实上游验收。
- `observed`：实际 Rust 安装器已访问官方 GitHub latest，在临时目录完成真实 Release 元数据读取、双摘要、下载、解压和唯一二进制检查并通过；测试结束自动清理。尚未执行该二进制，也未完成真实 socket、错误正文无落盘、外部连接、真实 API-key 上游或旧版运行回滚观测。

## 兼容性与限制

- 当前桌面正式支持范围仍是 macOS 13+ arm64；安装器包含 darwin/linux/windows 与 amd64/aarch64 资产映射，但不构成其他平台支持承诺。
- 本地代理支持 CLIProxyAPI OAuth 凭据池或 OpenAI-compatible API-key 上游；OpenAI/Claude/Gemini endpoint 与协议可用性仍取决于内核和上游配置，尚未完成真实 E2E。
- 只有用户在“Codex 配置”页显式点击应用或恢复时才会事务性修改 `config.toml`；本地代理启停不会自动切换路由，也不会改写 `auth.json`。
- 每次启动使用随机 runtime session，正常停止或下次 stopped 状态会清理配置；若应用被 `SIGKILL`，旧 sidecar 仍可能存活，下一次启动会因端口占用 fail closed，但尚未实现安全的跨进程 ownership/PID 回收。
- CLIProxyAPI `v7.2.145` 的 OAuth callback 可能监听全网卡，Management API 可读取敏感配置，因此本应用不开放这些控制面；导入文件仅由原生层投影到私有 auth-dir。这不是 EasyCLIProxyAPI 全量管理功能复制。
- 上游 `openai-compatibility` 无禁 redirect 或 DNS/IP pinning 配置，因此任意自定义 Base URL 的 SSRF/DNS rebinding 边界无法达到原 Rust 网关标准。这是架构级发布 No-Go，不能用一次运行观测替代。
- 上游预编译 Release 没有 Developer ID、公证、detached signature、SBOM 或可复现 provenance，并存在无法由当前配置关闭的后台版本请求。正式发布前必须建立独立审核 manifest/固定版本准入并完成网络、磁盘观测；直接跟随 latest 当前只属于技术原型。

## 隐私与安全

- provider API Key 的静态来源仍是应用专用 Keychain；运行时文件是 CLIProxyAPI 读取凭据所必需的明确例外，权限固定 `0600` 并在停止时删除。
- 官方 OAuth token、消息正文、Authorization、Cookie、query、原始响应和 Management key 不进入 sidecar 配置、SQLite、普通 DTO 或应用日志。
- Release 双 SHA-256 只能证明下载字节与同一 GitHub 信任域发布信息一致，不能替代独立签名或 provenance。

## 发布状态

- `published`：否；没有 tag、Release、latest.json 或 updater 通道变化。
- `accepted`：否；尚缺真实预编译内核运行、安全观测、真实 OAuth/API-key 上游 E2E、跨进程 ownership 恢复和独立上游版本准入。
- `cleanup`：真实下载安装测试只使用自动删除的临时目录；staging 已清理，未运行第三方二进制，也未创建应用数据目录中的 core/runtime 数据。

## 完整性

本切片没有新版本号、tag、exact source SHA 或发布资产。CLIProxyAPI 审计基线为上游 `v7.2.145`、tag commit `d9cea8904b14fbbebb77ef26e98ef08f6b48a724`；该事实不代表 Codex Manager 已分发或接受该二进制。
