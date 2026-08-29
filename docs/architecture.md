# 架构说明

## 产品边界

Codex Manager 是独立的 Tauri 2 桌面应用，不是 Codex 插件或反向代理。macOS 是首发平台；规范化、存储、价格、项目发现和 AGENTS 解析放在 Rust/SQLite 边界内，为后续 Windows 适配保留平台接口。

```text
Codex Desktop / CLI
  ├─ rollout JSONL ─────── Rollout adapter ─┐
  ├─ opt-in OTLP/HTTP ──── OTel adapter ────┼─ Normalizer / provenance
  ├─ config.toml ───────── AGENTS config ───┤
  └─ codex app-server ──── Capability probe ┘
                                             │
                                  SQLite + checkpoint/revision
                                             │
                                      explicit Tauri IPC
                                             │
                                        React desktop UI

User click ─ GitHub latest.json ─ Rust updater + embedded public key ─ native confirm ─ signed app replacement

OAuth UI ─ explicit IPC ─ bounded one-shot App Server account adapter ─ account/read + account/rateLimits/read
User click ─ explicit IPC ─ exact `codex login` subprocess ─ system browser ─ official Codex credential store
Native picker ─ bounded JSON ─ isolated official validation ─ app-specific macOS Keychain profiles
User confirm ─ config/read=file ─ revision/CAS/process lock ─ atomic `auth.json` replace ─ official post-switch verification
```

## 采集层

### Rollout adapter

- 只扫描用户配置 Codex home 的 `sessions` 和 `archived_sessions`。
- canonical path 必须仍位于允许目录；拒绝符号链接和非普通文件。
- 以设备/文件身份、UTF-8 byte offset 和 parser resume state 增量读取。
- 单行上限 4 MiB；EOF 未完成行不提交，但仍计入读取预算；超长未结束行不从中间 checkpoint，避免把攻击者选定的后缀当成新事件。
- 候选枚举按目录名新到旧并最多占用 5 秒，随后按活动 `sessions` 优先、同来源 mtime 新到旧、路径稳定兜底排序；整轮仍受 20 秒、256 MiB、200,000 events 等安全预算约束，但枚举或归档积压不能先耗尽整轮预算而使正在写入的会话长期饥饿。
- 文件截断或身份变化时，仅重建该来源的可重建投影。
- watcher 只使用一个合并信号槽，满时丢弃重复唤醒，并每 60 秒 reconciliation。每轮同时限制遍历数、字节、事件和时间，达到预算时显示 `partial/degraded`。
- 成功扫描后后端只发送不含数据的本地刷新事件；前端以 2 秒窗口合并事件。活动页每 5 秒只读取 `ingest_files.updated_at` 与 OTel `received_at` 组成的采集水位，只在水位变化时重载当前页。后台重载不执行活动总数 `COUNT`；慢请求期间最多合并一次后续刷新，不重跑扫描或全局 bootstrap。用户点击“刷新本地数据”时会先显式执行同一有预算的 reconciliation，扫描完成后在保留表格的同时刷新当前活动查询与总数。
- 活动查询依托 logical fingerprint 索引以 anti-join 选出每个逻辑记录的稳定代表，避免每次刷新对全表做 `ROW_NUMBER` 窗口排序；列表或总览数据与其采集水位在同一个 SQLite 只读事务快照内读取，独立 OTel 写连接不能造成旧数据配新水位。总览的记录/成功/失败数使用一次聚合，但价格仍按单次 model call 分桶，不会为性能改变计价口径。
- 启动 bootstrap 不再同步执行全量总览汇总；它先返回最新活动、项目、来源与配置，前端首屏可用后再单独请求 dashboard。汇总期间指标显示 `unavailable`/后台汇总说明，不会以 0 伪装已完成统计。

JSONL 是兼容适配源，不被描述为稳定公开 API。新的事件源必须实现 adapter，UI 不直接理解私有 JSON 结构。

### OTel adapter

- 用户显式启用后只绑定 `127.0.0.1` 的首次随机、后续持久化端口；自签 localhost 证书提供服务器身份，每个请求还必须在 body 解码前通过定时比较的专用 header 认证。持久化端口被占用时 fail closed，不自动切换到 Codex 未配置的端口。
- 支持 OTLP/HTTP protobuf 与 JSON；请求体上限 128 KiB，最多 16 条并发 TLS 连接、4 个并发请求，连接每 10 秒无读写进展即释放；另限制频率、3 秒处理时间和 resource/scope/event/attribute/data-point 数量。一批最多 256 条，单事务落库，超配额整批拒绝。
- 日志 body 从不读取。只物化固定 allowlist：未知 event/provider 折叠为 `other`，已知价格模型保留标识，未知 model/thread/turn 仅持久化带类型的 SHA-256 截断伪名，不保存未受控的自由文本。
- endpoint 在落盘前只保留固定 provider/origin 分类和已知 route 枚举；原始 host、租户子域、path、username、password、query 与 fragment 均不保留。
- 活动列表只把 `logs/codex.api_request` 作为请求级记录。聚合 metrics 不展开成虚假的逐请求行；`codex.sse_event` token 也不在缺少可靠关联键时猜测合并。

### App Server capability

应用从 Desktop bundle、显式环境变量或 PATH 寻找受信任的 Codex 可执行文件，使用有超时、无 stdin 的子进程在应用私有临时目录生成 JSON Schema，并对有数量、单文件与总大小上限的产物做 SHA-256 指纹。它只证明本机 CLI 支持对应命令，不代表已经连接或订阅 Codex Desktop 内部 app-server。

OAuth 不复用宽松的 capability 候选信任边界。macOS beta 只读取固定 ChatGPT bundle 的 Codex 源，将其复制到随机 `0700` 临时目录后再用系统 `codesign` 同时要求 identifier `codex` 与 OpenAI Developer ID Team ID `2DC432GLL2`；执行的是复制后验签的稳定副本，并在应用生命周期内复用。每次启动前复验路径；App Server 使用 `POSIX_SPAWN_START_SUSPENDED` 在用户代码执行前暂停，再以 `+PID` 动态验证实际进程，验签通过后才 `SIGCONT`。待导入 token 只会在进程恢复后以 zeroizing 内存请求通过 stdio 交给该 PID，因此 PATH shim、脚本、环境覆盖和验签后路径替换不能读取待导入 token。非 macOS 当前返回 `unavailable`。后端为该二进制启动独立、短生命周期的 App Server stdio 进程，依次完成 `initialize`、按需的 `account/login/start` `chatgptAuthTokens`、`account/read`、`config/read` 与 `account/rateLimits/read`；仅导入验证在 `initialize` 中显式选择 `capabilities.experimentalApi=true`，协议不兼容时 fail closed。App Server JSON-RPC 阶段预算为 15 秒，限制 128 条消息、每行 256 KiB 和最多 16 个额度桶，只把有界白名单 DTO 返回 WebView。RAII process guard 覆盖早退路径，stdout reader 的回收等待另有 1 秒上限。原始 JSON、token、授权/回调 URL 与错误响应正文不会返回或持久化。用户点击登录时只启动精确的 `codex login` 子命令；独立于页面轮询的后端 supervisor 会在 10 分钟内等待退出或强制 kill/wait，不自行实现 PKCE、回调监听、授权码交换或凭据存储。

OAuth 与认证 App Server 不继承任意父进程环境，只保留 HOME、用户、临时目录、语言环境、固定系统 PATH 与后端受控的 `CODEX_HOME`。官方支持的 refresh/revoke endpoint override、login client override、proxy、CA、API key 和 access token 变量全部被剔除，避免把导入验证流量或凭据导向调用者指定的端点。

认证档案是独立于 SQLite 的平台秘密存储。macOS 后端通过原生选择器读取一个普通 JSON 文件，检查当前 uid、权限不宽于 `0600`、单硬链接、128 KiB、读取前后 fstat、重复键、最大深度/字段数和 ChatGPT OAuth 结构；动态验签并恢复官方 App Server 后，只以 `chatgptAuthTokens` 外部认证提交 access token 与工作区 ID，执行账户/额度探测并确认身份。refresh token 不参与在线探测，也不会生成临时 `auth.json`。验证后的 bytes 使用 zeroizing buffer 写入应用专用 Keychain；Keychain 内部索引持有 email 与工作区 account id 组合的不可逆指纹，用于去重与切换复核，但 WebView DTO 只有随机 id、本地 label、状态和时间。软删除档案在 Keychain 保留 30 天，活动档案和最后一个可用档案不可删除。

账户切换不实现代理或请求调度。Rust 从后端解析 Codex home，只接受当前用户、单硬链接且权限不宽于 `0600` 的 file 模式 `auth.json`；官方 App Server 的 `config/read` 还必须证明最终生效的 `cli_auth_credentials_store` 明确为 `file`，缺失、keyring 或 auto 都 fail closed。写操作经同进程互斥、Manager 实例间 `flock`、用户原生确认和短期 opaque revision；后端复核 SHA-256、mtime、大小、权限与文件身份后，在同目录以强制 `0600` 的临时文件原子替换，并用官方 App Server 验证活动账户。账户复核如触发官方凭据刷新，后端最多重试三次，只有前后文件戳稳定时才使用刷新后的 bytes 更新 Keychain 或回滚。任何外部改写或身份不符均拒绝；后验证或 Keychain 状态提交失败时只在仍能证明文件属于本次切换的条件下回滚。官方 CLI/IDE 不参与该 `flock` 协议，因此删除在提交前额外复核，切换/删除确认也要求先结束正在运行的 Codex 任务。

## 规范化与指标口径

- 一个 `token_count` 快照不天然等于一个新网络请求。
- 去重键组合 session、ordinal、事件类型与累计 token 向量。累计向量不变表示重复快照；同一 turn 内非单调回退隔离为诊断。
- 新 turn 的首个累计向量回退可作为计数器重置。显式 last usage 只在与累计向量或可验证差值一致时接受；矛盾、溢出或不可能的 token 桶会进入诊断并标记 `unavailable`，不使用未验证数值。
- 单次 model call 的每个 token 字段上限为 10,000,000；累计快照字段上限为 1,000,000,000,000，两层都校验非负、`cached + cache-write <= input`、`reasoning <= output` 和 `total = input + output`。
- 普通 input、cached input、cache-write input 是互斥计费桶；reasoning output 属于 output，不二次收费。
- 长上下文倍率按单次 model call 判断，不按汇总后的跨请求 token 判断。
- 每个展示字段携带 source/confidence；缺失值是 `unavailable`，不是 0。
- rollout 的 `time_to_first_token_ms` 是 Codex-reported first visible output，不宣称为代理层网络 TTFB。
- API 等价价格使用带生效日期的内置目录；未知模型或桶不完整时整项不可用。

## 存储与并发

SQLite 位于平台应用数据目录，启用 WAL、foreign keys、busy timeout、页数硬上限和单后端写入。Unix 上应用数据目录与数据库分别收紧为 `0700` 与 `0600`。Rollout 投影以稳定文件身份（不可得时为路径）生成内部 source namespace；对外仍显示 Codex 原 session/event id，但不允许另一来源转移数据所有权。另外为 turn 和 model call 计算不含所有权的 logical fingerprint；字节等价的 rollout 副本在列表、总览和价格查询时折叠，存储与重建仍完全隔离。

主要投影：

- `ingest_files`：checkpoint、稳定文件身份、resume state、错误和未解析计数。
- `sessions`、`turns`、`model_calls`、`timeline_items`：规范化 rollout 元数据；timeline 只保存消息 UTF-8 长度，不保存正文。
- `otel_events`：allowlist 后的 OTel 元数据。
- `projects`：canonical 项目路径、来源、Git/worktree 状态。
- `agents_revisions`：两阶段 pending/applied revision，用于崩溃恢复、冲突保护和回滚。
- `settings`：Codex homes、授权根目录、保留期、OTel 开关和价格目录版本。

保留期会清理旧 turn/session 与 OTel 记录；OTel 硬删除只依据本机 `received_at`，不信任客户端时间。AGENTS revision 不受该保留期影响，而是每个文件最多保留 20 版。

## AGENTS 解析与安全写入

解析行为与 Codex 文档保持同一方向：

1. 全局目录优先取首个非空 `AGENTS.override.md` 或 `AGENTS.md`。
2. 从项目根到选定 cwd，逐目录按 override、AGENTS、配置的 fallback 顺序取首个非空文件。
3. 合并到 `project_doc_max_bytes` 后停止；应用另设 2 MiB 硬读取上限。

前端首次加载项目链时优先打开 Codex home 顶级 `AGENTS.md`，不存在时再回退到其他全局或项目文件；未授予写权限时仍可只读展示。项目状态点使用独立的 `hasAgentsFile` 字段，只在 canonical 项目根目录存在非符号链接普通 `AGENTS.md` 时为绿色；嵌套文件、fallback 与 `AGENTS.override.md` 仍计入发现数量，但不会冒充根文件存在。

发现文件不等于写授权。写入必须同时通过已知项目、显式授权根目录、canonical containment、文件名 allowlist、逐段 no-follow、安全文件身份、expected SHA/mtime 和最大大小检查。Unix 上从 `/` 开始打开根目录链，每个组件均通过 `openat(O_DIRECTORY | O_NOFOLLOW)` 验证；随后在同一目录创建临时文件、fsync，并在 atomic swap 后以同一安全路径重验 CAS，冲突时原子回滚，最后提交 revision。应用不会执行或信任 AGENTS 内容中的命令。

## 前端与 IPC

React 只使用 27 个显式 Tauri commands；其中 OTel 凭据只在用户点击显示配置时返回前端。OAuth/认证档案只获得账户摘要、登录状态、档案白名单 DTO 与变更结果；原生导入和确认由 Rust dialog 完成，WebView 没有路径、JSON 或 token。Tauri capability 清单不授予通用 shell、任意文件系统、HTTP、process、dialog 或 updater 插件 API，WebView 使用严格 CSP。浏览器开发模式使用人工构造的 demo adapter，不读取本机 Codex 数据。

## 在线更新信任边界

更新源和 minisign 公钥固定在 Tauri 配置中。WebView 只能调用 `check_for_update` 和 `install_pending_update`：前者返回当前/可用版本、日期和最多 4,000 字符的纯文本说明，不返回 URL、签名或原始 JSON；后者只能消费 Rust 内存中上一次成功检查所得的同一 `Update` 对象，期望版本不一致或应用重启后都必须重新检查。清单检查超时为 30 秒，已检查对象的下载/安装超时为 10 分钟；两者共用单飞行锁，下载/验签/安装失败不保留 pending update。

网络请求由 Rust updater 执行，不放宽 WebView `connect-src`。安装前通过 Rust dialog 插件显示原生确认框；前端未被授予 dialog 或 updater 插件权限。Tauri 更新签名用于应用内包验证，Developer ID、Hardened Runtime、Apple notarization 和 stapling 用于 macOS Gatekeeper 信任，两条链都必须通过。

## 跨平台边界

跨平台：event normalization、SQLite schema、价格、项目模型、AGENTS precedence、DTO 与 React UI。

平台适配：可执行文件发现、稳定文件身份、文件 no-follow/原子替换、应用数据目录、密钥库、原生文件选择/确认和打包签名。macOS 使用 `openat`/`O_NOFOLLOW`、Keychain 与 `flock`；Windows 当前只有部分可编译的 portable fallback，认证档案直接显示不可用。Windows beta 前必须补齐等价 credential vault、reparse-point/原子替换安全验证和安装包测试。

## 官方依据

- [Codex Advanced Configuration](https://learn.chatgpt.com/docs/config-file/config-advanced)
- [Codex Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Codex Authentication](https://developers.openai.com/codex/auth)
- [Codex App Server](https://developers.openai.com/codex/app-server)
- [Custom instructions with AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- [OpenAI API Pricing](https://learn.chatgpt.com/docs/pricing)
