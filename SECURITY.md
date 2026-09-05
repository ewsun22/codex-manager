# 安全策略

## 支持状态

项目目前是 macOS beta。源码和明确标注的 `Unsigned Community Build` prerelease 可供开发者验证，但它没有 Apple 身份保证，不属于正式支持的 macOS beta artifact。签名、公证的稳定下载通道已经建立；每个新版本仍须用自己的 exact SHA 重新通过全部门禁，不能继承旧版本的签名、公证或公开复验证据。Windows 尚未进入安全支持范围。

## 报告问题

公开仓库建立后，将在这里公布私密漏洞报告渠道和受支持版本。在此之前，请直接联系维护者，不要在公开 Issue 中提交 API Key、OAuth token、真实消息、AGENTS 内容、完整本机路径或未脱敏日志。

建议报告包含：受影响版本或 commit、macOS 版本、最小复现、预期/实际结果和不含秘密的诊断。不要利用漏洞读取、修改或上传不属于你的数据。

## 安全基线

- local-first、metadata-only、OTel opt-in；可选 CLIProxyAPI sidecar 默认停止，生成配置强制 IPv4 loopback，不提供系统代理、LAN listener 或 root helper。
- 不持久化 Codex 消息正文、Authorization、Cookie、OAuth code、完整环境变量或请求 query。API Key 与 CLIProxyAPI OAuth 秘密静态保存在各自隔离的 Keychain；sidecar 运行时才写入应用私有 `0700` 目录中的 `0600` 文件，并在 checkpoint 后删除。它们不得进入 SQLite、普通 WebView DTO、日志、备份或错误详情。
- OTel 只监听 IPv4 loopback 的首次随机、后续持久化端口，使用 localhost TLS 身份和 body 解码前专用 header 认证；持久化端口被占用时 fail closed。请求体上限 128 KiB，连接 10 秒无读写进展即超时，并限制连接/请求并发、频率、处理时间、解码基数与属性 allowlist；日志 body 不被读取。
- OTel event/provider 仅保存固定枚举，未知 model/thread/turn 仅保存带类型的 SHA-256 截断伪名，endpoint 仅保存 provider/origin/route 分类，避免将攻击者控制的高基数字符串持久化。
- Tauri WebView 使用严格 CSP，IPC 只暴露显式 command，不授予通用 shell、任意文件、HTTP、process、dialog 或 updater 插件 API。
- 桌面版在线更新检查在启动后按需运行，并默认每 12 小时自动运行一次，间隔可配置为 1–168 小时；Rust 只访问固定 GitHub `latest.json` 和固定 Tauri 公钥，自动检查仅持久化最近检查尝试时间，以及最近成功检查的当前/可用版本、发布日期和截断后的 notes，不保存 URL、签名对象、下载句柄、错误详情或凭据。成功或失败都从最近尝试起遵守配置间隔；失败不打断用户并保留上次成功状态。安装只消费同一次检查所得的内存中 `Update`，要求用户显式操作、同进程重新检查、版本一致、原生确认、minisign 验证和安装后重启；浏览器 demo 不自动联网。
- rollout/config 从文件系统根开始通过授权目录链逐段 no-follow 打开普通文件，身份、大小与内容来自同一句柄；限制遍历、字节、事件、时间和 4 MiB 单行大小。
- 不同 rollout 文件使用独立 source namespace 防止所有权转移；内容等价副本只在读取查询时通过 logical fingerprint 折叠。token 桶还要通过数值上限和语义一致性检查，矛盾数据标记为 `unavailable`。
- AGENTS 写入同时要求已知项目与显式授权根目录，并执行 canonical containment、逐段 no-follow、文件名 allowlist、SHA/mtime CAS、同目录临时文件、fsync 与原子替换。
- AGENTS 内容只作为文本显示和保存，绝不会当作本应用权限指令或命令执行。
- capability probe 只运行受信任候选 Codex，stdin 关闭、时间受限，Schema 文件数、单文件大小和总大小受限。
- OAuth 状态只通过一个短生命周期官方 App Server 读取：macOS 只从固定 ChatGPT bundle 复制 Codex 到随机私有目录，并在复制后验证 identifier `codex` 与 OpenAI Developer ID Team ID；每次启动前复验缓存路径，App Server 再以 `POSIX_SPAWN_START_SUSPENDED` 暂停启动，用无路径歧义的 `+PID` 动态验证实际进程后才 `SIGCONT`。签名验证上限 5 秒，App Server JSON-RPC 阶段上限 15 秒，256 KiB 单行、128 条消息、16 个额度桶，并在 Rust 中白名单化后才进入 WebView。所有早退路径均由 RAII 回收 child，reader 等待另有 1 秒上限。用户触发登录只执行精确的 `codex login`，由独立后端 supervisor 在 10 分钟内退出或强制回收；不提供通用命令参数、通用 HTTP 或授权 URL/回调粘贴。
- 官方订阅首次成功读取后，可将已经过 Rust 白名单化的 `CodexAccountSnapshot` 另存到应用专用 Keychain，供当前 OAuth 页面 cache-first 读取。快照可含 email、套餐、额度窗口和确认时间，但不含 access/refresh token、Cookie、授权 URL、原始 App Server JSON 或错误正文，也不进入 SQLite。超过 6 小时或强制刷新失败时只以显式 `stale` 显示；可信 CLI 临时缺失或复验失败只允许回退读取该快照，不会放宽登录、导入、切换或撤销操作的 executable 验证。
- OAuth/App Server 子进程使用固定环境白名单与系统 PATH；只保留 HOME、用户、临时目录、语言环境和受控 `CODEX_HOME`。refresh/revoke endpoint、login client、proxy、CA、API key 和 access token 等环境覆盖不会继承到凭据边界。
- 认证档案导入完全位于原生后端：文件选择器不把路径或 bytes 返回 WebView；普通文件/当前 uid/权限不宽于 `0600`/单硬链接/128 KiB/读取前后 fstat/JSON 重复键/深度与字段数均受限。实际 App Server PID 动态验签并恢复后，后端才以 zeroizing 请求通过官方 `chatgptAuthTokens` 流程提交 access token 和工作区 ID，refresh token 不交给在线探测，也不创建临时 `auth.json`。验证后的秘密以 zeroizing buffer 处理并存入应用专用 Keychain，IPC 只返回白名单元数据。
- 活动账户切换只支持安全的 file 模式 `auth.json`。App Server `config/read` 必须证明最终生效的 `cli_auth_credentials_store` 明确为 `file`；同进程异步门闩、Codex Manager 实例间非阻塞锁、opaque revision、SHA/mtime/文件身份 CAS、`0600` 同目录原子替换、切换后官方账户复核与条件回滚共同防止并发覆盖。活动账户读取如触发官方凭据刷新，后端必须在文件稳定后重新取得 bytes 和文件戳，再用于 Keychain 更新或回滚。官方登录子进程的整个生命周期也持有 Manager 进程锁。keyring/auto、配置缺失、外部改写、身份不符、活动/最后档案删除均 fail closed；删除提交前再次读取活动文件，软删除保留 30 天后才清理。CLI/IDE 不遵守 Manager 的锁，用户仍须结束正在运行的 Codex 任务。
- 三个信任域严格隔离：官方 OAuth access/refresh token 不得进入 CLIProxyAPI；CLIProxyAPI OAuth 只来自原生文件选择器，存入独立 Keychain service `cc.codex.manager.cliproxy-auth.v1`。普通 DTO 不含账户、路径、token 或 fingerprint；Management API、OAuth callback、原始配置、认证文件、`api-call` 与动态插件不得通过 WebView 开放。
- CLIProxyAPI 配置固定 `host: 127.0.0.1` 和非空随机 client key；同时强制 `commercial-mode:true`、`request-log:false`、`debug:false`、`logging-to-file:false`、`usage-statistics-enabled:false`、remote management/control panel/plugins disabled。启动验收必须检查受管 PID 与 `/healthz`，端口占用不得终止无关进程。
- 内测只安装已在代码中审核锁定的 `router-for-me/CLIProxyAPI v7.2.145` 平台/架构资产，使用内置精确 SHA-256 验证，不查询或自动追随上游 `latest`；下载与解压有上限，拒绝绝对路径、`..`、symlink/hardlink，经私有 staging 安装。安装后额外记录已解压二进制摘要，状态读取和启动前复核；不匹配时界面提供切回审核基线的入口。确认和安装只能由用户显式触发。
- 本地代理只承载 CLIProxyAPI OAuth 凭据池；外部 API Key 供应商由“Codex 配置”直接管理，不经过 CLIProxyAPI。CLIProxyAPI 的输入和运行时目录仍固定在 IPv4 loopback、随机 client key、私有 `0700` 目录和 `0600` 文件权限内。
- SQLite 在 Unix 上使用私有目录/文件权限、WAL、foreign keys、busy timeout 和全库页数硬上限；OTel 另有总行数配额与单事务批量上限。
- CI action 以完整 commit SHA 固定；发布流程包含 lockfile、fail-closed 审计、无 `rg` 依赖且未完整扫描即失败的 secret scan、SBOM、许可证清单与 artifact SHA-256。

## 已知边界

- rollout JSONL 是内部兼容源；畸形或未来事件会降级 source health，不能被视为稳定安全协议。
- macOS no-follow/原子替换路径已实现；Windows portable fallback 尚未完成 reparse-point 等价安全验证。
- unsigned 构建没有 Apple 身份保证，Gatekeeper 警告是预期行为。它只能作为 GitHub prerelease 的 `Unsigned Community Build` 分发，并且不得携带稳定 updater manifest。只有 Developer ID 签名、Apple accepted、公证 ticket stapled 并验证后，才能称为正式 macOS beta artifact。
- SQLite 和普通应用数据对同一 OS 用户可见；认证档案秘密不进入这些位置，而是使用应用专用 macOS Keychain。Keychain 访问仍服从当前登录用户和系统的访问控制，未提供额外主密码。
- 账户白名单快照虽然不含 credential，仍可能包含 email 和额度信息；它与认证档案 secret 使用不同的 Keychain service/account，删除应用数据或退出官方账户不会自动等价于删除该快照，后续应提供用户可见的显式清理入口。
- 官方 Codex 可能按用户自己的 `cli_auth_credentials_store` 设置把活动凭据保存在 OS credential store 或 `~/.codex/auth.json`。多档案切换不会改写该设置，只支持 file 模式；keyring/auto 会拒绝切换。切换活动文件会影响共享该缓存的 CLI 与 IDE 扩展。
- 当前轮换是用户确认后的整账户显式切换，不是请求级调度；不会检测限额后后台自动换号，也不会迁移正在运行的会话。
- Codex 配置页面只管理 `model`、`model_provider` 和一个 provider table，使用 `model_providers.<id>.auth.command` 调用当前 app binary 的 allowlisted opaque secret ref；私有 journal 支持 CAS/原子应用与恢复，文件漂移拒绝覆盖。`verified` 只代表写后文件复核，不代表 Codex 成功请求上游。真实 CLIProxyAPI OAuth E2E、上游 5xx、崩溃恢复与旧内核回滚尚未 `accepted`；2026-09-06 的隔离空池观测已验证 IPv4 loopback、人工 4xx 请求未落盘标记和正常停止清理，详见 [S5–S6 记录](docs/validation/reliability-s5-s6.md)。
- 每次 sidecar 使用随机 runtime session；正常停止会 kill/wait 自己持有的 child 并删除整个 session，下次 stopped 状态会清理崩溃遗留配置，启动前的端口独占检查会拒绝把旧 `/healthz` 误认成新 child。但 macOS 在应用被 `SIGKILL` 时没有可靠 parent-death signal，旧 CLIProxyAPI 仍可能存活；在持久化 ownership/PID/启动时间并可安全回收前，本应用只 fail closed，不按进程名广泛 kill。
- CLIProxyAPI `v7.2.145` 上游 Release 的 Developer ID、公证、detached signature、SBOM 和可复现 provenance 不由本项目继承；当前内测使用代码内置的精确资产 SHA-256 拒绝字节漂移，后续在审阅新版本后更新基线。上游构建可能拉取 mutable model catalog，属于已知供应链限制和后续优化项，不再作为内测发布阻断。
- 预编译 CLIProxyAPI 的 OAuth callback 可能绑定全网卡，且存在无法由当前配置关闭的 Antigravity 外部版本请求。因此本应用不开放其 Management API/OAuth callback；导入的 OAuth 文件仅由原生层投影到私有 auth-dir。2026-09-06 已完成一次隔离空凭据池的网络与磁盘观测；该证据不覆盖真实 OAuth 成功路径，后续每版不重复已通过的空池观测，除非内核版本或配置边界发生变化。
- 当前没有远程撤回机制。桌面版会在启动后按需检查并按配置间隔自动检查更新；自动检查不下载、不安装、不重启。在线更新会将完整更新包读入内存后验签；严格下载字节上限是后续加固项。
- 签名发布已拆成 secret-free build、无 Release 写权限的受保护 sign、无 Apple/updater secret 的 draft 三个 job；sign/draft 不执行 build artifact 携带的 signer 或 verifier。sign 先下载固定版本 Tauri signer并核对 SHA-512，updater 私钥只在紧邻签名命令的独立 step 注入并在命令后 unset；验签使用 exact checkout 中仅依赖 Node 标准密码库的 Minisign/Ed25519 verifier，transfer 只携带公开 updater key。临时证书/Keychain 在任何上传前清理。draft 只允许 exact source/Release/asset digest 调和，所有资产按 ID 回下载复验；人工发布后另由只读 workflow 匿名验证正式 tag URL 和 latest alias。发布人员已报告 updater 私钥存在独立离线备份，首次发布前仍须复核实际恢复介质。

## 已完成的安全验证

2026-08-27 完成一次全代码树安全扫描，确认的 12 项发现（9 medium、3 low）已全部修复，并由新的只读验证者逐项追踪为 `verified_fixed`。这是当时 unversioned working tree 的本机证据，不替代正式发布 commit 的重新扫描、签名与公证。

## 发布安全门（内测精简版）

内测 prerelease 只要求针对当前 exact SHA 完成应用测试、secret scan、CLIProxyAPI `v7.2.145` 精确版本与 SHA-256 校验、产物完整性检查，并明确披露 `unsigned/unnotarized` 状态；不再把 SBOM、许可证全集、上游 provenance、跨进程完美回收或每个 provider 的真实网络链路作为内测阻断条件。内测包不得进入稳定 updater 通道。

正式 macOS 发布仍要求 Developer ID、Hardened Runtime、Apple notarization、stapling、Gatekeeper 和 updater 签名；这些是平台分发要求，不等同于 CLIProxyAPI 上游必须具备 Developer ID、SBOM 或可复现构建。正式发布前至少完成一次真实 CLIProxyAPI OAuth → loopback → Codex Responses E2E，并记录停止、凭据 checkpoint 和失败清理结果。未完成的真实 E2E 只能标为 `tested locally`/`unaccepted`，不能写成 `accepted`。

v0.6.6 的限定发布例外：用户在已明确获知真实 OAuth 尚未验收后，授权“提交，推送，releases”。本次按已披露限制执行签名发布，不把真实 OAuth 业务标为 `accepted`，也不以发布授权代替真实账户、配置切换或计费调用授权；旧版 updater 安装测试按用户要求跳过。平台签名、公证、Gatekeeper、updater 签名和草稿/公开资产复验门禁不变。这个例外仅适用于 v0.6.6，不自动改变以后版本的发布条件。

## 调试验收入口

`desktop-qa` 仅允许 debug 构建；release 同时启用该 feature 会编译失败。其独立 Builder 仅注册人工数据验收所需的活动、项目和 AGENTS 命令，认证 backend 禁用，不加载 updater，真实账户、配置应用、代理、设置修改和 CLI probe 均无 handler。不得把该入口扩展为生产配置或凭据绕过手段。默认发行构建保持现有命令权限与三个凭据信任域。
