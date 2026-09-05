# 隐私说明

Codex Manager 是本地优先应用。默认不上传数据，也不提供产品遥测。

## 应用读取什么

- 配置的 Codex home 下 `sessions` 与 `archived_sessions` 中的 rollout JSONL。
- 已观测项目和用户明确授权根目录中的项目标记与受支持 AGENTS 文件。
- Codex home 的 `config.toml` 中与 AGENTS 解析相关的 `project_doc_max_bytes` 和 `project_doc_fallback_filenames`；配置文件仅在内存解析，不写入数据库。
- 用户显式开启 receiver 后，发往当前 `https://localhost:<persisted-random-port>/v1/logs` 或 `/v1/metrics` 的已认证 OTLP/HTTP 请求。
- 用户主动运行 capability probe 时，本机 Codex 可执行文件的版本和生成的 App Server Schema。
- 首次打开 OAuth 页面或用户主动刷新时，官方 App Server 返回的认证方式、email、套餐和 ChatGPT Codex 额度窗口；成功后白名单摘要与时间可进入独立的 macOS Keychain 缓存，供后续 cache-first 展示。
- 用户点击导入后，由 Rust 原生文件选择器读取的一个本地 ChatGPT OAuth JSON；路径与原始内容不会交给 WebView，文件须为当前用户拥有、权限不宽于 `0600`、单硬链接且不超过 128 KiB 的普通文件，读取前后会复核文件身份与时间。
- 导入验证不创建临时 `auth.json`。实际 App Server PID 通过动态验签后，后端仅在 zeroizing 内存请求中向短生命官方进程提交 access token 和工作区 ID；refresh token 不交给在线探测。
- 用户在“供应商与反代”保存的供应商名称、Base URL、模型、reasoning effort 和 write-only API Key；API Key 只由原生后端接收，不会从读取命令返回 WebView。
- 用户显式启动本地代理后，应用会从独立 Keychain 读取所选 CLIProxyAPI OAuth 档案，投影到随机私有 `0700` auth-dir/`0600` 文件并启动官方预编译 sidecar；发送到本机 loopback 的请求由该 sidecar 处理。外部 API Key 供应商由 Codex 配置直接使用。

## 应用持久化什么

- session/turn 标识、时间、项目路径、模型、provider、推理等级、token 分类、耗时、结果和可信度。
- 采集 checkpoint、稳定文件身份、未解析计数和已脱敏错误分类。
- 项目 canonical path、Git/worktree 状态、授权路径设置和保留期。
- OTel 固定 allowlist 元数据。event/provider/origin/route 在持久化前转为有界枚举；已知价格目录模型保留标识，未知 model/thread/turn 只保存带类型的 SHA-256 截断伪名。原始 host/path、租户子域、username、password、query 和 fragment 不落库。
- 价格目录版本和 API 等价估算所需的 token 元数据。
- 用户通过应用创建、保存或恢复 AGENTS 文件时的 revision。revision 包含保存前后的 AGENTS 文本，每个文件最多保留 20 版。
- 用户明确导入的认证档案秘密与内部索引，保存在应用专用 macOS Keychain 服务 `cc.codex.manager.auth-profiles.v1`。界面可见的档案元数据只有随机 id、本地名称、活动/删除状态与创建/更新时间；不含 email、文件路径、文件名、token 或账户指纹，也不写入 SQLite。
- Codex provider 的随机 id、名称、规范化 Base URL、模型、reasoning effort、创建/更新时间，以及用户选择的本机端口。API Key 与 CLIProxyAPI OAuth 秘密分别保存在隔离的应用专用 macOS Keychain item，不进入 SQLite；CLIProxyAPI sidecar 运行期间仅将 OAuth 凭据投影到应用私有 `0700` runtime 的 `0600` auth 文件，checkpoint 后删除。
- 官方订阅 cache-first 快照仅允许账户摘要（可含 email）、套餐、额度窗口和确认/更新时间进入独立 Keychain，并作为不含 token/原始响应的白名单 DTO供当前 OAuth 页面读取；不进入 SQLite。
- CLIProxyAPI OAuth 档案的秘密与内部索引进入独立 Keychain service `cc.codex.manager.cliproxy-auth.v1`；支持格式为 `codex`、`claude`、`antigravity`、`kimi`、`xai`。普通 DTO 只含随机 id、label、provider、启用状态、时间和受限状态码，不含账户、路径、token 或 fingerprint；identity 只用于本地防串号，不是联网验真。

数据库位于操作系统为 `cc.codex.manager` 分配的本机应用数据目录。Unix 系统上应用会把数据目录与数据库权限分别收紧为 `0700` 和 `0600`。

## 应用不持久化什么

- Codex 用户消息、助手消息、reasoning 文本、tool arguments 或 tool output 正文。
- OTel log body、prompt、error message、未知属性或未受控的 event/provider 自由文本。
- Authorization、Cookie、API Key、OAuth code、完整环境变量、请求 header、请求 query 或 URL fragment。
- OAuth access token、refresh token、完整授权/回调 URL、`auth.json` 内容、原始 App Server JSON 和未筛选的 OAuth 页账户/额度响应不会进入 SQLite、WebView、应用日志或崩溃消息。cache-first 只保存明确白名单摘要、套餐、额度窗口及时间；用户明确导入的原始凭据仅在原生后端受限内存中验证，并在通过后保存到应用专用 Keychain。
- 供应商 API Key、CLIProxyAPI OAuth token、反代请求/响应 body、header、query、完整上游错误正文和用户内容不会进入 SQLite、普通 WebView DTO、应用日志或备份；CLIProxyAPI 运行期间唯一落盘例外是 sidecar 所必需的私有 `0600` auth 文件，停止前 checkpoint 后删除。
- Codex Schema 生成目录；capability probe 完成后临时目录自动清理，只保存版本与 SHA-256。
- CLI Schema 兼容性探测只保存最近探测的版本、状态与 Schema 指纹；它是非采集源，不保存原始 Schema 正文，也不代表连接 Codex Desktop 内部 App Server。

timeline 中只保存消息正文的 UTF-8 byte 长度。解析器和 OTel receiver 的测试数据均为人工构造 fixture，不包含真实会话。

## 网络与监听

- 默认不监听端口。用户在设置中明确开启 OTel receiver 后，它首次只在 IPv4 loopback 随机分配空闲端口，然后在应用私有配置中持久化并复用。持久化端口被占用时安全失败，不自动改用其他端口。
- receiver 使用应用私有目录中的 localhost TLS 身份：Codex exporter 通过私有 CA 校验 receiver 身份，receiver 在读取/解码 body 前通过专用 header token 认证调用方。它拒绝带 `Origin` 的浏览器请求，请求体上限 128 KiB，空闲连接 10 秒超时，并限制连接/请求并发、频率、处理时间和解码后基数。专用 token 只在用户点击显示配置时进入 WebView，不写入管理器数据库或日志。
- 应用不会安装 CA、系统代理、root helper 或系统服务，也不会自动修改 Codex 配置。
- 只有用户点击 OAuth 登录按钮时，应用才启动官方 `codex login`。系统浏览器到 OpenAI 的认证、回调、凭据刷新与存储由官方 Codex 负责；该操作可能改变 CLI 与 IDE 扩展共享的当前账户。Codex Manager 不启动自有回调监听器，也不接收授权码或 token。
- 认证档案导入、切换、删除和恢复也只在用户点击后运行。官方切换只支持权限受限的 file 模式 `auth.json`，CLIProxyAPI 档案只走原生选择器和独立 Keychain；不跨域共享 token，不按请求自动轮换或云同步。
- CLIProxyAPI 反代默认停止，只有用户选择 OAuth 池并点击开启后才启动受管 sidecar；生成配置固定 `127.0.0.1`，关闭远程管理、控制面板、插件、usage statistics 和请求日志，同时启用 `commercial-mode`。应用不开放 CLIProxyAPI Management API 或 OAuth callback，不自动安装 CA、系统代理、服务或 root helper。
- OAuth 模式启动时投影随机私有 auth-dir；正常停止先将内核原地 refresh 后的 provider/identity/CAS checkpoint 回独立 Keychain，再精确清理。发现 pending 崩溃证据时 fail closed，确认端口无 orphan 后才恢复。
- 首页提供本地代理状态、OpenAI/Claude/Gemini 兼容接入地址和显式开关；首页不展示 bearer 或 API Key。实际协议可用性取决于内核与上游配置。
- 用户显式安装或首次开启反代时，应用只会访问已审核锁定的 CLIProxyAPI `v7.2.145` 官方 GitHub tag 资产地址；GitHub 会常规看到网络元数据。应用不请求上游 `latest`，不会上传 Codex 消息、项目或凭据。下载字节与内置精确 SHA-256 不一致、资产超限或解压越界时拒绝安装。
- 桌面版启动后按需检查更新，并默认每 12 小时自动检查一次（设置可配置 1–168 小时）；Rust 后端只访问固定的 `https://github.com/ewsun22/codex-manager/releases/latest/download/latest.json`。自动检查仅取得并持久化最近检查尝试时间，以及最近成功检查的当前/可用版本、发布日期和截断后的 notes，不保存 URL、签名对象、下载句柄、错误详情或凭据；成功或失败都从最近尝试起遵守配置间隔，失败保留上次成功状态且不打断使用。下载、签名校验、安装和重启仍需用户显式操作；浏览器 demo 不自动联网。不会上传 Codex 消息、项目、AGENTS 或本地设置；GitHub 作为下载服务会常规看到网络请求元数据。
- 本项目没有内置云同步、远程分析或自动上传。构建和依赖安装阶段访问包仓库不属于运行时数据上传。

## 保留与删除

设置中的保留天数用于清理旧 turn/session 与 OTel 元数据，范围为 1–3650 天。AGENTS revision 使用独立策略：每个文件保留最近 20 版，不随活动保留期删除。官方认证档案与 CLIProxyAPI OAuth 档案使用各自的软删除/恢复策略；Keychain secret 不提供导出。已安装的 CLIProxyAPI 二进制、版本元数据和上一版回滚目录不受活动保留天数影响；运行配置在 checkpoint 后删除。

v0.6.6 在启动和修改保留期时更新清理截止时间，持续运行时按小时推进；没有可监听目录时仍保留定期维护。截止时间约束每次导入事务，首次导入、文件重建和重复扫描不会恢复已过期的任务。清理在同一事务中删除过期且已观测终态的任务、关联调用和 timeline 元数据，并更新活动刷新标记；保留增量 checkpoint。未观测终态或缺少可用时间戳的任务暂不自动清理，避免中断仍可能继续的任务。OTel 继续以本机 `received_at` 清理，不信任客户端时间。以上清理仅作用于 Manager 数据库，不删除源 rollout 文件。

AGENTS 未保存草稿仅存在于当前页面内存，不进入 localStorage、SQLite 或遥测；离开有修改的编辑器需要明确放弃确认。后端冲突保护与每文件 20 版 revision 策略继续生效。

当前 UI 尚未提供“一键清空全部应用数据”。用户可退出应用后删除操作系统应用数据目录；正式稳定版前应补充可发现的数据目录入口与确认式清理功能。

## 用户责任

项目路径、模型标识、session id 和 AGENTS 内容本身也可能是敏感信息。只应授权可信根目录；公开问题报告、截图或日志前应自行脱敏。不要在 AGENTS 文件或 Issue 中放置密钥。

## 界面语言偏好

界面语言和“跟随设备”选择只保存在应用 WebView 的本地存储中，不进入 SQLite、日志、备份、Tauri 后端 DTO 或任何 Keychain。语言判断只读取设备公开的浏览器 locale，不读取消息、凭据或请求内容。

## 开发验收数据

显式启用的 `desktop-qa` 调试构建只生成私有临时人工 SQLite、Codex home 与项目文件，WebView 为非持久 store，认证 backend 禁用且不提供账户/代理/updater 命令。验收记录和截图仅使用人工内容；默认发布构建不启用该入口。独立内核 smoke 观测使用空认证目录和临时随机本机 key，不调用真实模型或读取用户 Keychain。
