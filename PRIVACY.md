# 隐私说明

Codex Manager 是本地优先应用。默认不上传数据，也不提供产品遥测。

## 应用读取什么

- 配置的 Codex home 下 `sessions` 与 `archived_sessions` 中的 rollout JSONL。
- 已观测项目和用户明确授权根目录中的项目标记与受支持 AGENTS 文件。
- Codex home 的 `config.toml` 中与 AGENTS 解析相关的 `project_doc_max_bytes` 和 `project_doc_fallback_filenames`；配置文件仅在内存解析，不写入数据库。
- 用户显式开启 receiver 后，发往当前 `https://localhost:<persisted-random-port>/v1/logs` 或 `/v1/metrics` 的已认证 OTLP/HTTP 请求。
- 用户主动运行 capability probe 时，本机 Codex 可执行文件的版本和生成的 App Server Schema。

## 应用持久化什么

- session/turn 标识、时间、项目路径、模型、provider、推理等级、token 分类、耗时、结果和可信度。
- 采集 checkpoint、稳定文件身份、未解析计数和已脱敏错误分类。
- 项目 canonical path、Git/worktree 状态、授权路径设置和保留期。
- OTel 固定 allowlist 元数据。event/provider/origin/route 在持久化前转为有界枚举；已知价格目录模型保留标识，未知 model/thread/turn 只保存带类型的 SHA-256 截断伪名。原始 host/path、租户子域、username、password、query 和 fragment 不落库。
- 价格目录版本和 API 等价估算所需的 token 元数据。
- 用户通过应用创建、保存或恢复 AGENTS 文件时的 revision。revision 包含保存前后的 AGENTS 文本，每个文件最多保留 20 版。

数据库位于操作系统为 `cc.codex.manager` 分配的本机应用数据目录。Unix 系统上应用会把数据目录与数据库权限分别收紧为 `0700` 和 `0600`。

## 应用不持久化什么

- Codex 用户消息、助手消息、reasoning 文本、tool arguments 或 tool output 正文。
- OTel log body、prompt、error message、未知属性或未受控的 event/provider 自由文本。
- Authorization、Cookie、API Key、OAuth code、完整环境变量、请求 header、请求 query 或 URL fragment。
- Codex Schema 生成目录；capability probe 完成后临时目录自动清理，只保存版本与 SHA-256。

timeline 中只保存消息正文的 UTF-8 byte 长度。解析器和 OTel receiver 的测试数据均为人工构造 fixture，不包含真实会话。

## 网络与监听

- 默认不监听端口。用户在设置中明确开启后，receiver 首次只在 IPv4 loopback 随机分配空闲端口，然后在应用私有配置中持久化并复用。持久化端口被占用时安全失败，不自动改用其他端口。
- receiver 使用应用私有目录中的 localhost TLS 身份：Codex exporter 通过私有 CA 校验 receiver 身份，receiver 在读取/解码 body 前通过专用 header token 认证调用方。它拒绝带 `Origin` 的浏览器请求，请求体上限 128 KiB，空闲连接 10 秒超时，并限制连接/请求并发、频率、处理时间和解码后基数。专用 token 只在用户点击显示配置时进入 WebView，不写入管理器数据库或日志。
- 应用不会安装 CA、系统代理、root helper 或系统服务，也不会自动修改 Codex 配置。
- 本项目没有内置云同步、远程分析或自动上传。构建和依赖安装阶段访问包仓库不属于运行时数据上传。

## 保留与删除

设置中的保留天数用于清理旧 turn/session 与 OTel 元数据，范围为 1–3650 天。AGENTS revision 使用独立策略：每个文件保留最近 20 版，不随活动保留期删除。

当前 UI 尚未提供“一键清空全部应用数据”。用户可退出应用后删除操作系统应用数据目录；正式公开 beta 前应补充可发现的数据目录入口与确认式清理功能。

## 用户责任

项目路径、模型标识、session id 和 AGENTS 内容本身也可能是敏感信息。只应授权可信根目录；公开问题报告、截图或日志前应自行脱敏。不要在 AGENTS 文件或 Issue 中放置密钥。
