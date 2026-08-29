# 隐私说明

Codex Manager 是本地优先应用。默认不上传数据，也不提供产品遥测。

## 应用读取什么

- 配置的 Codex home 下 `sessions` 与 `archived_sessions` 中的 rollout JSONL。
- 已观测项目和用户明确授权根目录中的项目标记与受支持 AGENTS 文件。
- Codex home 的 `config.toml` 中与 AGENTS 解析相关的 `project_doc_max_bytes` 和 `project_doc_fallback_filenames`；配置文件仅在内存解析，不写入数据库。
- 用户显式开启 receiver 后，发往当前 `https://localhost:<persisted-random-port>/v1/logs` 或 `/v1/metrics` 的已认证 OTLP/HTTP 请求。
- 用户主动运行 capability probe 时，本机 Codex 可执行文件的版本和生成的 App Server Schema。
- 打开 OAuth 页面或主动刷新时，官方 App Server 返回的认证方式、email、套餐和 ChatGPT Codex 额度窗口；这些字段只用于当前界面，不写入数据库。
- 用户点击导入后，由 Rust 原生文件选择器读取的一个本地 ChatGPT OAuth JSON；路径与原始内容不会交给 WebView，文件须为当前用户拥有、权限不宽于 `0600`、单硬链接且不超过 128 KiB 的普通文件，读取前后会复核文件身份与时间。
- 导入验证不创建临时 `auth.json`。实际 App Server PID 通过动态验签后，后端仅在 zeroizing 内存请求中向短生命官方进程提交 access token 和工作区 ID；refresh token 不交给在线探测。
- 用户在“供应商与反代”保存的供应商名称、Base URL、模型、reasoning effort 和 write-only API Key；API Key 只由原生后端接收，不会从读取命令返回 WebView。
- 用户显式启动网关后，应用会在内存中读取该供应商 API Key，并接收用户主动发送到本机 loopback 的 Responses 请求，再转发到该供应商。

## 应用持久化什么

- session/turn 标识、时间、项目路径、模型、provider、推理等级、token 分类、耗时、结果和可信度。
- 采集 checkpoint、稳定文件身份、未解析计数和已脱敏错误分类。
- 项目 canonical path、Git/worktree 状态、授权路径设置和保留期。
- OTel 固定 allowlist 元数据。event/provider/origin/route 在持久化前转为有界枚举；已知价格目录模型保留标识，未知 model/thread/turn 只保存带类型的 SHA-256 截断伪名。原始 host/path、租户子域、username、password、query 和 fragment 不落库。
- 价格目录版本和 API 等价估算所需的 token 元数据。
- 用户通过应用创建、保存或恢复 AGENTS 文件时的 revision。revision 包含保存前后的 AGENTS 文本，每个文件最多保留 20 版。
- 用户明确导入的认证档案秘密与内部索引，保存在应用专用 macOS Keychain 服务 `cc.codex.manager.auth-profiles.v1`。界面可见的档案元数据只有随机 id、本地名称、活动/删除状态与创建/更新时间；不含 email、文件路径、文件名、token 或账户指纹，也不写入 SQLite。
- Codex provider 的随机 id、名称、规范化 Base URL、模型、reasoning effort、创建/更新时间，以及用户选择的本机端口。供应商 API Key 与随机本机 bearer 分别保存在应用专用 macOS Keychain item，不进入 SQLite。

数据库位于操作系统为 `cc.codex.manager` 分配的本机应用数据目录。Unix 系统上应用会把数据目录与数据库权限分别收紧为 `0700` 和 `0600`。

## 应用不持久化什么

- Codex 用户消息、助手消息、reasoning 文本、tool arguments 或 tool output 正文。
- OTel log body、prompt、error message、未知属性或未受控的 event/provider 自由文本。
- Authorization、Cookie、API Key、OAuth code、完整环境变量、请求 header、请求 query 或 URL fragment。
- OAuth access token、refresh token、完整授权/回调 URL、`auth.json` 内容、原始 App Server JSON 和 OAuth 页账户/额度快照不会进入 SQLite、WebView、应用日志或崩溃消息。用户明确导入的原始凭据仅在原生后端受限内存中验证，并在通过后保存到应用专用 Keychain。
- 供应商 API Key、本机网关 bearer、网关请求/响应 body、header、query、完整上游错误正文和用户内容不会进入 SQLite、普通 WebView DTO、应用日志或备份。敏感 bearer 只在用户通过原生确认后生成的临时配置片段中进入 WebView。
- Codex Schema 生成目录；capability probe 完成后临时目录自动清理，只保存版本与 SHA-256。

timeline 中只保存消息正文的 UTF-8 byte 长度。解析器和 OTel receiver 的测试数据均为人工构造 fixture，不包含真实会话。

## 网络与监听

- 默认不监听端口。用户在设置中明确开启 OTel receiver 后，它首次只在 IPv4 loopback 随机分配空闲端口，然后在应用私有配置中持久化并复用。持久化端口被占用时安全失败，不自动改用其他端口。
- receiver 使用应用私有目录中的 localhost TLS 身份：Codex exporter 通过私有 CA 校验 receiver 身份，receiver 在读取/解码 body 前通过专用 header token 认证调用方。它拒绝带 `Origin` 的浏览器请求，请求体上限 128 KiB，空闲连接 10 秒超时，并限制连接/请求并发、频率、处理时间和解码后基数。专用 token 只在用户点击显示配置时进入 WebView，不写入管理器数据库或日志。
- 应用不会安装 CA、系统代理、root helper 或系统服务，也不会自动修改 Codex 配置。
- 只有用户点击 OAuth 登录按钮时，应用才启动官方 `codex login`。系统浏览器到 OpenAI 的认证、回调、凭据刷新与存储由官方 Codex 负责；该操作可能改变 CLI 与 IDE 扩展共享的当前账户。Codex Manager 不启动自有回调监听器，也不接收授权码或 token。
- 认证档案导入、切换、删除和恢复也只在用户点击后运行。切换只支持权限受限的 file 模式 `auth.json`，由后端以冲突检查和原子替换完成；不按请求自动轮换，不提供 token 代理或云同步。
- Codex Responses 网关同样默认停止，只有用户选择 API-key 供应商并点击启动后才绑定 `127.0.0.1`。它只转发用户发往该 loopback 端点的请求到所选供应商；所选供应商会按自身隐私条款看到请求内容与常规网络元数据。应用不记录 body/header/query，不把官方 OAuth token 交给网关，不自动修改 Codex 配置，也不安装 CA、系统代理、服务或 root helper。
- 桌面版启动后按需检查更新，并默认每 12 小时自动检查一次（设置可配置 1–168 小时）；Rust 后端只访问固定的 `https://github.com/ewsun22/codex-manager/releases/latest/download/latest.json`。自动检查仅取得并持久化最近检查尝试时间，以及最近成功检查的当前/可用版本、发布日期和截断后的 notes，不保存 URL、签名对象、下载句柄、错误详情或凭据；成功或失败都从最近尝试起遵守配置间隔，失败保留上次成功状态且不打断使用。下载、签名校验、安装和重启仍需用户显式操作；浏览器 demo 不自动联网。不会上传 Codex 消息、项目、AGENTS 或本地设置；GitHub 作为下载服务会常规看到网络请求元数据。
- 本项目没有内置云同步、远程分析或自动上传。构建和依赖安装阶段访问包仓库不属于运行时数据上传。

## 保留与删除

设置中的保留天数用于清理旧 turn/session 与 OTel 元数据，范围为 1–3650 天。AGENTS revision 使用独立策略：每个文件保留最近 20 版，不随活动保留期删除。认证档案删除使用另一独立策略：不能删除活动档案或最后一个可用档案；其他档案先在 Keychain 内软删除并保留 30 天，读取档案列表时会清理过期项目，保留期内可恢复。删除 provider 会同时删除对应 API Key；本机网关 bearer 作为安装级 Keychain secret 保留，当前 UI 不提供导出或单独清理入口。

当前 UI 尚未提供“一键清空全部应用数据”。用户可退出应用后删除操作系统应用数据目录；正式稳定版前应补充可发现的数据目录入口与确认式清理功能。

## 用户责任

项目路径、模型标识、session id 和 AGENTS 内容本身也可能是敏感信息。只应授权可信根目录；公开问题报告、截图或日志前应自行脱敏。不要在 AGENTS 文件或 Issue 中放置密钥。
