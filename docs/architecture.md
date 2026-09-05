# 架构说明

## 产品边界

Codex Manager 是独立的 Tauri 2 桌面应用，不是 Codex 插件、系统代理或通用 API 网关。v0.6.1 已把旧 Rust 进程内 Responses 转发器替换为默认停止、独立版本、IPv4 loopback-only 的 CLIProxyAPI 预编译 sidecar；官方订阅仍是独立的受信任 CLI/App Server/Keychain 边界。macOS 是首发平台；规范化、存储、价格、项目发现和 AGENTS 解析放在 Rust/SQLite 边界内，为后续 Windows 适配保留平台接口。

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

Desktop startup / interval ─ GitHub latest.json ─ Rust metadata check ─ sidebar/card reminder ─ User click ─ native confirm ─ signed app replacement

OAuth UI ─ explicit IPC ─ bounded one-shot App Server account adapter ─ account/read + account/rateLimits/read
User click ─ explicit IPC ─ exact `codex login` subprocess ─ system browser ─ official Codex credential store
Native picker ─ bounded JSON ─ isolated official validation ─ app-specific macOS Keychain profiles
User confirm ─ config/read=file ─ revision/CAS/process lock ─ atomic `auth.json` replace ─ official post-switch verification

Provider UI ─ write-only API Key IPC ─ Keychain secret + SQLite metadata
User install ─ official CLIProxyAPI Release ─ digest + checksums ─ private staging/core
User start ─ private 0600 auth-dir ─ managed CLIProxyAPI sidecar ─ OAuth upstream
Native confirm ─ temporary Codex provider snippet; no automatic config/auth mutation
```

## 采集层

### Rollout adapter

- 只扫描用户配置 Codex home 的 `sessions` 和 `archived_sessions`。
- canonical path 必须仍位于允许目录；拒绝符号链接和非普通文件。
- 以设备/文件身份、UTF-8 byte offset 和 parser resume state 增量读取。
- 单行上限 4 MiB；EOF 未完成行不提交，但仍计入读取预算；超长未结束行不从中间 checkpoint，避免把攻击者选定的后缀当成新事件。
- v0.6.6 的 S5 使用有界变更队列及跨轮目录游标：连续三个变更文件名额后保留一个后台名额，后台在单文件续读和历史发现之间交替；名额计数不因一轮预算结束而重置。发现每轮最多 4,096 个 entry、累计 100 ms，处理最多 4,096 个文件；整轮仍受 20 秒、256 MiB、200,000 events 限制，单文件批次仍限制 32 MiB / 20,000 events。
- 初次发现按 `sessions` 根优先于 `archived_sessions` 进行，不再为全局 mtime 排序收集全部文件；冷启动未变化的历史文件不保证最新优先。实际文件变更可插入下一优先名额，历史最终覆盖不依赖每次从目录起点重扫。深度上限仍为 16；目录句柄上限 17，变更及续读队列各最多 1,024 个去重路径。
- 文件截断或身份变化时，仅重建该来源的可重建投影。
- watcher 保留一个合并唤醒槽及独立有界路径 inbox；访问类事件忽略，文件变化优先处理，目录变化、队列溢出或事件错误请求 reconciliation。60 秒单调 deadline 不会被持续写入推迟；仍有工作时 250 ms 后继续有预算扫描，发现游标不被重复请求重置，达到预算显示 `partial/degraded`。完整周期仍覆盖丢失的事件。
- 成功扫描后后端只发送不含数据的本地刷新事件；前端以 2 秒窗口合并事件。活动页每 5 秒只读取包含 ingest、OTel 及删除 revision 的采集水位，只在水位变化时重载当前页。后台重载不执行活动总数 `COUNT`；慢请求期间最多合并一次后续刷新，不重跑扫描或全局 bootstrap。用户点击“刷新本地数据”时会先显式执行同一有预算的 reconciliation，扫描完成后在保留表格的同时刷新当前活动查询与总数。
- 活动查询依托 logical fingerprint 索引以 anti-join 选出每个逻辑记录的稳定代表，避免每次刷新对全表做 `ROW_NUMBER` 窗口排序；列表或总览数据与其采集水位在同一个 SQLite 只读事务快照内读取，独立 OTel 写连接不能造成旧数据配新水位。总览的记录/成功/失败数使用一次聚合，但价格仍按单次 model call 分桶，不会为性能改变计价口径。
- 启动 bootstrap 不再同步执行全量总览汇总；它先返回最新活动、项目、来源与配置，前端首屏可用后再单独请求 dashboard。汇总期间指标显示 `unavailable`/后台汇总说明，不会以 0 伪装已完成统计。

JSONL 是兼容适配源，不被描述为稳定公开 API。新的事件源必须实现 adapter，UI 不直接理解私有 JSON 结构。

### OTel adapter

- 用户显式启用后只绑定 `127.0.0.1` 的首次随机、后续持久化端口；自签 localhost 证书提供服务器身份，每个请求还必须在 body 解码前通过定时比较的专用 header 认证。持久化端口被占用时 fail closed，不自动切换到 Codex 未配置的端口。
- 支持 OTLP/HTTP protobuf 与 JSON；请求体上限 128 KiB，最多 16 条并发 TLS 连接、4 个并发请求，连接每 10 秒无读写进展即释放；另限制频率、3 秒处理时间和 resource/scope/event/attribute/data-point 数量。一批最多 256 条，单事务落库，超配额整批拒绝。
- 日志 body 从不读取。只物化固定 allowlist：未知 event/provider 折叠为 `other`，已知价格模型保留标识，未知 model/thread/turn 仅持久化带类型的 SHA-256 截断伪名，不保存未受控的自由文本。
- endpoint 在落盘前只保留固定 provider/origin 分类和已知 route 枚举；原始 host、租户子域、path、username、password、query 与 fragment 均不保留。
- 活动列表只把 `logs/codex.api_request` 作为请求级记录。聚合 metrics 不展开成虚假的逐请求行；`codex.sse_event` token 也不在缺少可靠关联键时猜测合并。

### CLI Schema 兼容性（非采集源）

应用从 Desktop bundle、显式环境变量或 PATH 寻找受信任的 Codex 可执行文件，使用有超时、无 stdin 的子进程在应用私有临时目录生成 JSON Schema，并对有数量、单文件与总大小上限的产物做 SHA-256 指纹。它只证明本机 CLI 支持对应命令，不代表已经连接或订阅 Codex Desktop 内部 app-server。最近探测的版本、状态和 Schema 指纹会持久化；原始 Schema 正文不持久化。该探测不是采集源，不影响 rollout 主采集。

OAuth 不复用宽松的 capability 候选信任边界。macOS beta 只读取固定 ChatGPT bundle 的 Codex 源，将其复制到随机 `0700` 临时目录后再用系统 `codesign` 同时要求 identifier `codex` 与 OpenAI Developer ID Team ID `2DC432GLL2`；执行的是复制后验签的稳定副本，并在应用生命周期内复用。每次启动前复验路径；App Server 使用 `POSIX_SPAWN_START_SUSPENDED` 在用户代码执行前暂停，再以 `+PID` 动态验证实际进程，验签通过后才 `SIGCONT`。待导入 token 只会在进程恢复后以 zeroizing 内存请求通过 stdio 交给该 PID，因此 PATH shim、脚本、环境覆盖和验签后路径替换不能读取待导入 token。非 macOS 当前返回 `unavailable`。后端为该二进制启动独立、短生命周期的 App Server stdio 进程，依次完成 `initialize`、按需的 `account/login/start` `chatgptAuthTokens`、`account/read`、`config/read` 与 `account/rateLimits/read`；仅导入验证在 `initialize` 中显式选择 `capabilities.experimentalApi=true`，协议不兼容时 fail closed。App Server JSON-RPC 阶段预算为 15 秒，限制 128 条消息、每行 256 KiB 和最多 16 个额度桶，只把有界白名单 DTO 返回 WebView。RAII process guard 覆盖早退路径，stdout reader 的回收等待另有 1 秒上限。原始 JSON、token、授权/回调 URL 与错误响应正文不会返回或持久化。用户点击登录时只启动精确的 `codex login` 子命令；独立于页面轮询的后端 supervisor 会在 10 分钟内等待退出或强制 kill/wait，不自行实现 PKCE、回调监听、授权码交换或凭据存储。

OAuth 与认证 App Server 不继承任意父进程环境，只保留 HOME、用户、临时目录、语言环境、固定系统 PATH 与后端受控的 `CODEX_HOME`。官方支持的 refresh/revoke endpoint override、login client override、proxy、CA、API key 和 access token 变量全部被剔除，避免把导入验证流量或凭据导向调用者指定的端点。

认证档案是独立于 SQLite 的平台秘密存储。macOS 后端通过原生选择器读取一个普通 JSON 文件，检查当前 uid、权限不宽于 `0600`、单硬链接、128 KiB、读取前后 fstat、重复键、最大深度/字段数和 ChatGPT OAuth 结构；动态验签并恢复官方 App Server 后，只以 `chatgptAuthTokens` 外部认证提交 access token 与工作区 ID，执行账户/额度探测并确认身份。refresh token 不参与在线探测，也不会生成临时 `auth.json`。验证后的 bytes 使用 zeroizing buffer 写入应用专用 Keychain；Keychain 内部索引持有 email 与工作区 account id 组合的不可逆指纹，用于去重与切换复核，但 WebView DTO 只有随机 id、本地 label、状态和时间。软删除档案在 Keychain 保留 30 天，活动档案和最后一个可用档案不可删除。

账户切换不实现请求调度，也不进入网关供应商池。Rust 从后端解析 Codex home，只接受当前用户、单硬链接且权限不宽于 `0600` 的 file 模式 `auth.json`；官方 App Server 的 `config/read` 还必须证明最终生效的 `cli_auth_credentials_store` 明确为 `file`，缺失、keyring 或 auto 都 fail closed。写操作经同进程互斥、Manager 实例间 `flock`、用户原生确认和短期 opaque revision；后端复核 SHA-256、mtime、大小、权限与文件身份后，在同目录以强制 `0600` 的临时文件原子替换，并用官方 App Server 验证活动账户。账户复核如触发官方凭据刷新，后端最多重试三次，只有前后文件戳稳定时才使用刷新后的 bytes 更新 Keychain 或回滚。任何外部改写或身份不符均拒绝；后验证或 Keychain 状态提交失败时只在仍能证明文件属于本次切换的条件下回滚。官方 CLI/IDE 不参与该 `flock` 协议，因此删除在提交前额外复核，切换/删除确认也要求先结束正在运行的 Codex 任务。

### Codex provider 与 CLIProxyAPI sidecar

`codex_providers` 只保存随机 id、名称、规范化 Base URL、模型、reasoning effort 与时间；供应商 API Key 保存在独立 Keychain service。保存命令把 API Key 视为 write-only input，普通 list/save DTO 只返回 `hasApiKey`。删除供应商时同时删除其 Keychain item；仍被任何 Codex 配置档案引用的供应商不可删除。

总览只显示本地代理的非秘密快捷状态、OpenAI/Claude/Gemini 兼容接入地址、内核版本、PID 和显式开关，不展示 bearer 或 API Key。代理只选择 CLIProxyAPI OAuth 池；多协议 endpoint 是内核 client 入口，实际可用性取决于 OAuth 凭据和内核。外部 API-key 上游由 Codex 配置编排器直接管理，不经过 sidecar。

桌面主体与 CLIProxyAPI 使用独立版本。内测安装器只允许用户显式确认和安装代码中已审核的 `v7.2.145`；Rust 按当前 OS/arch 选择内置精确资产名与 SHA-256，从官方 tag URL 下载，不请求或追随上游 `latest`。资产下载上限 128 MiB、解压上限 512 MiB；tar/zip 路径必须相对且无 `..`，链接与额外可执行文件拒绝。解压在应用私有 `0700` staging 中进行，校验完成后才切换 core；安装目录切换与健康失败回滚保护已安装版本，但完美跨进程恢复不作为内测发布门禁。应用不下载或编译 Go 源码。

用户显式启动 OAuth 凭据池后，Rust 从隔离的 Keychain service 读取秘密，为每次启动生成随机私有 `0700` runtime session，并投影随机 `auth-dir` 与 `0600` 文件。外部 API-key 上游由 Codex 配置编排器直接管理，不经过 sidecar。配置固定 `host: 127.0.0.1`、`commercial-mode:true`、非空 client key、日志/usage stats/retry 关闭，并禁用 remote management、control panel、plugins。启动前独占验证 loopback 端口，启动成功要求新 child 持续存活且 `/healthz` 返回成功。正常停止先 checkpoint 内核原地 refresh 后的 provider/identity/CAS，再精确删除 runtime；发现 pending 崩溃证据时 fail closed，确认无 orphan 后才恢复。状态 DTO 的 request/failed/in-flight 因 usage stats 关闭而返回 `unavailable`。

应用被 `SIGKILL` 或系统强制终止时，pending runtime/checkpoint 是恢复证据；下次启动不会盲删或按名称强杀进程，而是先验证端口、ownership 与 checkpoint 状态，无法证明安全归属就 fail closed。

CLIProxyAPI 只能从文件读取运行时凭据，因此 OAuth token 的静态来源是隔离 Keychain，运行期间物化到私有 auth-dir；外部 API Key 不进入 CLIProxyAPI。这些运行文件不得进入 SQLite、WebView、日志或备份；正常停止前先 checkpoint，再精确清理。Codex 配置由独立编排器使用 `auth.command` 管理，不改写 `auth.json`。

CLIProxyAPI Management API 与 OAuth callback 不向 WebView 开放：OAuth 档案仅通过原生文件选择器导入并存入 `cc.codex.manager.cliproxy-auth.v1`，支持当前已理解的 provider 格式；运行时按需投影，官方 OAuth 永不进入该域。本地代理不再接受任意外部 Base URL，因此删除了该路径上的 redirect/DNS/IP 安全门；外部 endpoint 由 Codex 配置直接管理。

预编译内核不是主应用签名信任链的一部分。内测固定 CLIProxyAPI `v7.2.145` 及每个资产的精确 SHA-256，用于防止下载字节漂移；上游缺少 Developer ID、公证、detached signature、SBOM 和可复现 provenance 属于已知限制，不作为内测阻断。2026-09-06 已完成隔离空凭据池的 loopback、请求拒绝与停止清理观测，详见 [S5–S6 验证记录](validation/reliability-s5-s6.md)；真实 OAuth checkpoint 和上游成功业务链仍待授权验收。

官方订阅页面采用 cache-first。首次成功的账户读取或用户点击刷新后，后端仅将白名单账户摘要（可含 email）、套餐、额度窗口和确认/更新时间保存到独立 macOS Keychain，并作为不含 token/原始响应的白名单 DTO 提供当前 OAuth 页面读取；后续页面访问先读取该缓存，cache miss 或刷新按钮才解析并验签可信 CLI、触发实时读取，因此 CLI 暂时缺失时仍能显示已有安全快照。快照超过 6 小时或强制刷新失败时标记为 stale，并保留上次确认时间。token、原始 App Server JSON、授权 URL 和错误正文永不进入缓存。

总览布局为紧凑工作台：最近活动只呈现一条，API 等价估算在首页显示两位小数，底层金额和覆盖口径不变。侧栏本地桌面模式显示构建元数据中的应用版本和构建时间（可选短 SHA），不使用启动时间冒充构建时间。Codex 配置页不再渲染全局提示词、插件与市场、会话管理三个未开放占位模块。

## 规范化与指标口径

- 一个 `token_count` 快照不天然等于一个新网络请求。
- 查询层明确区分 `turn` 任务汇总与 model-call 交互事件：前者承载任务终态、首个可见输出和总耗时，后者只承载已观测 token 事件并引用所属任务状态；没有可靠终态的任务为 `unobserved`，不得继续显示为 `running`。
- 去重键组合 session、ordinal、事件类型与累计 token 向量。累计向量不变表示重复快照；同一 turn 内非单调回退隔离为诊断。
- 新 turn 的首个累计向量回退可作为计数器重置。显式 last usage 只在与累计向量或可验证差值一致时接受；矛盾、溢出或不可能的 token 桶会进入诊断并标记 `unavailable`，不使用未验证数值。
- 单次 model call 的每个 token 字段上限为 10,000,000；累计快照字段上限为 1,000,000,000,000，两层都校验非负、`cached + cache-write <= input`、`reasoning <= output` 和 `total = input + output`。
- 普通 input、cached input、cache-write input 是互斥计费桶；reasoning output 属于 output，不二次收费。
- 长上下文倍率按单次 model call 判断，不按汇总后的跨请求 token 判断。
- 每个展示字段携带 source/confidence；缺失值是 `unavailable`，不是 0。
- rollout 的 `time_to_first_token_ms` 是 Codex-reported first visible output，不宣称为代理层网络 TTFB。
- API 等价价格使用带生效日期的内置目录；未知模型或桶不完整的调用单独计入未覆盖，不参与金额。
- 总览按 canonical model call 逐调用计价，同时返回 priced calls/tokens 与 observed calls/tokens 覆盖率；部分不可计价数据不会使整张估算卡变成不可用，也不会按零值或默认模型补算。
- 项目 `lastConversationAt` 由该项目已观测 turn 的最后更新时间聚合，独立于项目重新发现时间；空值明确为 `unavailable`。

## 存储与并发

### v0.6.6 完整性与增量一致性修复

任务级 token 可用性同时取决于已观测调用和解析器的任务完整性标记。任务出现异常快照后，活动汇总不再把此前的部分调用提升为完整任务用量；有效调用仍可用于模型交互明细和有覆盖边界的估价。调用发生时明确记录的模型、provider、effort 保持原值；缺失字段在活动与成本查询中统一回退到对应任务，避免读取批次边界改变估价输入。查询修复可补全已有数据的缺失字段；已有非空调用模型保持原值，不追溯重判旧版本保存的字段，也不重写源 rollout。

`get_activity_facets` 返回所选活动视图全部记录的模型、推理级别候选和同一读取事务中的 revision，不接受任意 SQL 或文件路径。候选不依赖分页、当前模型或搜索条件；前端仍保留当前选择，切换视图时清理不适用的结果条件。

保留期截止时间及删除水位保存在独立 singleton 元数据表。截止时间在启动、修改设置和按小时维护时推进，每次 ingest 事务按已保存截止时间清理当前 source namespace 的过期终态任务，避免初次导入和重建复活旧数据。全库维护使用事务清理关联调用、timeline 与空会话，保持 checkpoint，并使活动 revision 感知删除。时间比较使用解析后的日期而非 RFC3339 字符串排序；未观测终态或没有可用时间的任务暂不自动删除。没有可监听目录时仍启动周期维护线程。

SQLite 位于平台应用数据目录，启用 WAL、foreign keys、busy timeout、页数硬上限和单后端写入。Unix 上应用数据目录与数据库分别收紧为 `0700` 与 `0600`。Rollout 投影以稳定文件身份（不可得时为路径）生成内部 source namespace；对外仍显示 Codex 原 session/event id，但不允许另一来源转移数据所有权。另外为 turn 和 model call 计算不含所有权的 logical fingerprint；字节等价的 rollout 副本在列表、总览和价格查询时折叠，存储与重建仍完全隔离。

主要投影：

- `ingest_files`：checkpoint、稳定文件身份、resume state、错误和未解析计数。
- `sessions`、`turns`、`model_calls`、`timeline_items`：规范化 rollout 元数据；timeline 只保存消息 UTF-8 长度，不保存正文。
- `otel_events`：allowlist 后的 OTel 元数据。
- `projects`：canonical 项目路径、来源、Git/worktree 状态。
- `agents_revisions`：两阶段 pending/applied revision，用于崩溃恢复、冲突保护和回滚。
- `settings`：Codex homes、授权根目录、保留期、OTel 开关、价格目录版本和更新检查间隔（1–168 小时，默认 12 小时）。更新检查状态只保存最近检查尝试时间，以及最近成功检查的受限展示元数据：当前/可用版本、发布日期和截断后的 notes。
- `codex_providers`：Codex 配置编排器与 API-key 上游的非秘密元数据；API Key、OAuth token 与运行时凭据不进入 SQLite。

保留期会清理旧 turn/session 与 OTel 记录；OTel 硬删除只依据本机 `received_at`，不信任客户端时间。AGENTS revision 不受该保留期影响，而是每个文件最多保留 20 版。

## AGENTS 解析与安全写入

解析行为与 Codex 文档保持同一方向：

1. 全局目录优先取首个非空 `AGENTS.override.md` 或 `AGENTS.md`。
2. 从项目根到选定 cwd，逐目录按 override、AGENTS、配置的 fallback 顺序取首个非空文件。
3. 合并到 `project_doc_max_bytes` 后停止；应用另设 2 MiB 硬读取上限。

前端首次加载项目链时优先打开 Codex home 顶级 `AGENTS.md`，不存在时再回退到其他全局或项目文件；未授予写权限时仍可只读展示。项目状态点使用独立的 `hasAgentsFile` 字段，只在 canonical 项目根目录存在非符号链接普通 `AGENTS.md` 时为绿色；嵌套文件、fallback 与 `AGENTS.override.md` 仍计入发现数量，但不会冒充根文件存在。

发现文件不等于写授权。写入必须同时通过已知项目、显式授权根目录、canonical containment、文件名 allowlist、逐段 no-follow、安全文件身份、expected SHA/mtime 和最大大小检查。Unix 上从 `/` 开始打开根目录链，每个组件均通过 `openat(O_DIRECTORY | O_NOFOLLOW)` 验证；随后在同一目录创建临时文件、fsync，并在 atomic swap 后以同一安全路径重验 CAS，冲突时原子回滚，最后提交 revision。应用不会执行或信任 AGENTS 内容中的命令。

## 前端与 IPC

v0.6.6 可靠性修复为项目/AGENTS 读取增加请求序号，迟到响应不能覆盖当前文件；未保存内容保留基准 SHA，离开前通过应用内确认对话框明确放弃，文件、项目、主导航与原生关闭路径异步等待同一决定；不依赖 macOS WebView 缺失的 `window.confirm` 实现。创建与恢复 AGENTS、Codex 配置应用/恢复/删除和本地代理 OAuth 档案移除也使用应用内确认；受限后端已有的原生确认继续保留。写入期间禁止继续编辑或导航。写入结果与修订/页面后续刷新分别反馈，刷新失败保留已成功写入的状态和可恢复内容。经过后端限定的字符串错误与 `Error` 通过统一转换提供操作提示，不回传原始凭据或任意网络响应。

主窗口限定授予 `core:window:allow-destroy`，供 Tauri `onCloseRequested` 在未保存保护允许关闭后完成其原有窗口销毁流程；不增加 shell、process、通用文件或 dialog 插件权限。新增 `get_activity_facets` 也须出现在构建命令表和主窗口精确 allow 权限中。

首页与本地代理页复用非秘密状态读取；页面可见时低频校验，并对并发读取去重。OAuth 档案不随状态轮询读取，仅在页面需要档案或显式变更后刷新。内核安装、启动、停止及相关阻塞文件/Keychain/进程操作在阻塞工作池执行；Tauri 入口保持异步和 transition 单飞行锁，gateway 锁只保护状态接管。启动健康检查覆盖整个等待循环的 10 秒截止期，每次请求最多 500ms，禁止重定向；超时后仍按受管进程结束、checkpoint 和回滚流程处理。扫描、项目发现和 CLI 探测同样移出 UI 线程。

React 只使用显式 Tauri commands；CLIProxyAPI 新增的能力仅限确认审核基线、安装审核基线、读取脱敏状态和启停受管进程。OTel 凭据只在用户点击显示配置并通过原生确认后进入专用配置片段；CLIProxyAPI client key 与 provider API Key 永不返回 WebView，前端只能获得非秘密 endpoint、运行状态和 `hasApiKey`。OAuth/认证档案只获得账户摘要、登录状态、档案白名单 DTO 与变更结果。原生导入和确认由 Rust dialog 完成，WebView 没有通用路径、JSON、HTTP、shell、process、dialog 或 updater 插件能力。WebView 使用严格 CSP；浏览器开发模式使用人工构造的 demo adapter，不读取本机 Codex 数据、下载内核或启动真实 listener。

## 在线更新信任边界

更新源和 minisign 公钥固定在 Tauri 配置中。桌面应用壳层在启动后根据最近尝试时间判断是否到期，并按设置的间隔（默认 12 小时，范围 1–168 小时）调用受限 Rust 检查命令；浏览器 demo 不联网。WebView 只能调用 `check_for_update` 和 `install_pending_update`：前者返回当前/可用版本、日期和最多 4,000 字符的纯文本说明，不返回 URL、签名或原始 JSON；后端持久化最近检查尝试时间，以及最近成功检查的当前/可用版本、发布日期和截断后的 notes，成功或失败都从最近尝试起遵守配置间隔。发现新版本时由前端在设置侧栏和应用更新卡片显示提醒；失败不打断用户且保留上次成功状态。后者只能消费 Rust 内存中上一次成功检查所得的同一 `Update` 对象，期望版本不一致或应用重启后都必须重新检查。清单检查超时为 30 秒，已检查对象的下载/安装超时为 10 分钟；两者共用单飞行锁，下载/验签/安装失败不保留 pending update。下载、签名校验、安装和重启不由自动检查触发，仍要求用户显式操作、同进程重新检查、版本核对和 macOS 原生确认。

网络请求由 Rust updater 执行，不放宽 WebView `connect-src`。安装前通过 Rust dialog 插件显示原生确认框；前端未被授予 dialog 或 updater 插件权限。Tauri 更新签名用于应用内包验证，Developer ID、Hardened Runtime、Apple notarization 和 stapling 用于 macOS Gatekeeper 信任，两条链都必须通过。

## 跨平台边界

跨平台：event normalization、SQLite schema、价格、项目模型、AGENTS precedence、provider DTO 与 React UI。

平台适配：可执行文件发现、稳定文件身份、文件 no-follow/原子替换、应用数据目录、密钥库、原生文件选择/确认和打包签名。macOS 使用 `openat`/`O_NOFOLLOW`、Keychain 与 `flock`；Windows 当前只有部分可编译的 portable fallback，认证档案与 provider secret 直接显示不可用。Windows beta 前必须补齐等价 credential vault、reparse-point/原子替换安全验证和安装包测试。

## 官方依据

- [Codex Advanced Configuration](https://learn.chatgpt.com/docs/config-file/config-advanced)
- [Codex Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Codex Authentication](https://developers.openai.com/codex/auth)
- [Codex App Server](https://developers.openai.com/codex/app-server)
- [Custom instructions with AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- [OpenAI API Pricing](https://learn.chatgpt.com/docs/pricing)

## 界面语言边界

React UI 通过源码级 `t()` / `tt` i18n 层提供 `zh-CN` 与 `en-US`，不扫描或改写已渲染 DOM。首次启动读取 WebView 的设备语言：`zh-*` 使用中文，其余语言使用英文；用户手动选择或恢复“跟随设备”仅写入版本化 localStorage key。语言偏好不进入 SQLite、Tauri IPC、日志或任何凭据域。数字、日期和金额格式跟随当前界面语言；项目名、模型标识、路径、外部 Release notes 和底层技术错误保持来源原文。

## 调试桌面验收

`desktop-qa` 是显式启用的 debug-only Cargo feature；release 与该 feature 同时启用时编译失败，默认产品入口不包含 QA 启动流程。隔离 Builder 使用人工 SQLite、Codex home、项目文件和非持久 WebView store，禁用认证 backend，只注册活动/项目/AGENTS 等必要安全命令；不注册账户、配置应用、代理、updater、设置修改或 CLI probe，不加载 updater。人工 DOM 驱动通过真实 Tauri IPC 进行保存/CAS/筛选和窗口生命周期验收，不替代真实 OAuth 或 updater 测试。
