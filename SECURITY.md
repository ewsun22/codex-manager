# 安全策略

## 支持状态

项目目前是 macOS beta 候选版。源码和明确标注的 `Unsigned Community Build` prerelease 可供开发者验证，但它没有 Apple 身份保证，不属于正式支持的 macOS beta artifact。签名、公证的稳定下载通道尚未建立；Windows 尚未进入安全支持范围。

## 报告问题

公开仓库建立后，将在这里公布私密漏洞报告渠道和受支持版本。在此之前，请直接联系维护者，不要在公开 Issue 中提交 API Key、OAuth token、真实消息、AGENTS 内容、完整本机路径或未脱敏日志。

建议报告包含：受影响版本或 commit、macOS 版本、最小复现、预期/实际结果和不含秘密的诊断。不要利用漏洞读取、修改或上传不属于你的数据。

## 安全基线

- local-only、metadata-only、OTel opt-in；不提供反向代理、系统代理或 root helper。
- 不持久化 Codex 消息正文、Authorization、Cookie、API Key、OAuth code、完整环境变量或请求 query。
- OTel 只监听 IPv4 loopback 的首次随机、后续持久化端口，使用 localhost TLS 身份和 body 解码前专用 header 认证；持久化端口被占用时 fail closed。请求体上限 128 KiB，连接 10 秒无读写进展即超时，并限制连接/请求并发、频率、处理时间、解码基数与属性 allowlist；日志 body 不被读取。
- OTel event/provider 仅保存固定枚举，未知 model/thread/turn 仅保存带类型的 SHA-256 截断伪名，endpoint 仅保存 provider/origin/route 分类，避免将攻击者控制的高基数字符串持久化。
- Tauri WebView 使用严格 CSP，IPC 只暴露显式 command，不授予通用 shell、任意文件、HTTP、process、dialog 或 updater 插件 API。
- 在线更新只由用户手动触发；Rust 固定 GitHub endpoint 和 Tauri 公钥，只返回有界纯文本元数据。安装只消费同一次检查所得的内存中 `Update`，要求版本一致、原生确认、minisign 验证和安装后重启。
- rollout/config 从文件系统根开始通过授权目录链逐段 no-follow 打开普通文件，身份、大小与内容来自同一句柄；限制遍历、字节、事件、时间和 4 MiB 单行大小。
- 不同 rollout 文件使用独立 source namespace 防止所有权转移；内容等价副本只在读取查询时通过 logical fingerprint 折叠。token 桶还要通过数值上限和语义一致性检查，矛盾数据标记为 `unavailable`。
- AGENTS 写入同时要求已知项目与显式授权根目录，并执行 canonical containment、逐段 no-follow、文件名 allowlist、SHA/mtime CAS、同目录临时文件、fsync 与原子替换。
- AGENTS 内容只作为文本显示和保存，绝不会当作本应用权限指令或命令执行。
- capability probe 只运行受信任候选 Codex，stdin 关闭、时间受限，Schema 文件数、单文件大小和总大小受限。
- OAuth 状态只通过一个短生命周期官方 App Server 读取：macOS 只从固定 ChatGPT bundle 复制 Codex 到随机私有目录，并在复制后验证 identifier `codex` 与 OpenAI Developer ID Team ID；每次启动前复验缓存路径，App Server 再以 `POSIX_SPAWN_START_SUSPENDED` 暂停启动，用无路径歧义的 `+PID` 动态验证实际进程后才 `SIGCONT`。签名验证上限 5 秒，App Server JSON-RPC 阶段上限 15 秒，256 KiB 单行、128 条消息、16 个额度桶，并在 Rust 中白名单化后才进入 WebView。所有早退路径均由 RAII 回收 child，reader 等待另有 1 秒上限。用户触发登录只执行精确的 `codex login`，由独立后端 supervisor 在 10 分钟内退出或强制回收；不提供通用命令参数、通用 HTTP 或授权 URL/回调粘贴。
- OAuth/App Server 子进程使用固定环境白名单与系统 PATH；只保留 HOME、用户、临时目录、语言环境和受控 `CODEX_HOME`。refresh/revoke endpoint、login client、proxy、CA、API key 和 access token 等环境覆盖不会继承到凭据边界。
- 认证档案导入完全位于原生后端：文件选择器不把路径或 bytes 返回 WebView；普通文件/当前 uid/权限不宽于 `0600`/单硬链接/128 KiB/读取前后 fstat/JSON 重复键/深度与字段数均受限。实际 App Server PID 动态验签并恢复后，后端才以 zeroizing 请求通过官方 `chatgptAuthTokens` 流程提交 access token 和工作区 ID，refresh token 不交给在线探测，也不创建临时 `auth.json`。验证后的秘密以 zeroizing buffer 处理并存入应用专用 Keychain，IPC 只返回白名单元数据。
- 活动账户切换只支持安全的 file 模式 `auth.json`。App Server `config/read` 必须证明最终生效的 `cli_auth_credentials_store` 明确为 `file`；同进程异步门闩、Codex Manager 实例间非阻塞锁、opaque revision、SHA/mtime/文件身份 CAS、`0600` 同目录原子替换、切换后官方账户复核与条件回滚共同防止并发覆盖。活动账户读取如触发官方凭据刷新，后端必须在文件稳定后重新取得 bytes 和文件戳，再用于 Keychain 更新或回滚。官方登录子进程的整个生命周期也持有 Manager 进程锁。keyring/auto、配置缺失、外部改写、身份不符、活动/最后档案删除均 fail closed；删除提交前再次读取活动文件，软删除保留 30 天后才清理。CLI/IDE 不遵守 Manager 的锁，用户仍须结束正在运行的 Codex 任务。
- SQLite 在 Unix 上使用私有目录/文件权限、WAL、foreign keys、busy timeout 和全库页数硬上限；OTel 另有总行数配额与单事务批量上限。
- CI action 以完整 commit SHA 固定；发布流程包含 lockfile、审计、secret scan、SBOM、许可证清单与 artifact SHA-256。

## 已知边界

- rollout JSONL 是内部兼容源；畸形或未来事件会降级 source health，不能被视为稳定安全协议。
- macOS no-follow/原子替换路径已实现；Windows portable fallback 尚未完成 reparse-point 等价安全验证。
- unsigned 构建没有 Apple 身份保证，Gatekeeper 警告是预期行为。它只能作为 GitHub prerelease 的 `Unsigned Community Build` 分发，并且不得携带稳定 updater manifest。只有 Developer ID 签名、Apple accepted、公证 ticket stapled 并验证后，才能称为正式 macOS beta artifact。
- SQLite 和普通应用数据对同一 OS 用户可见；认证档案秘密不进入这些位置，而是使用应用专用 macOS Keychain。Keychain 访问仍服从当前登录用户和系统的访问控制，未提供额外主密码。
- 官方 Codex 可能按用户自己的 `cli_auth_credentials_store` 设置把活动凭据保存在 OS credential store 或 `~/.codex/auth.json`。多档案切换不会改写该设置，只支持 file 模式；keyring/auto 会拒绝切换。切换活动文件会影响共享该缓存的 CLI 与 IDE 扩展。
- 当前轮换是用户确认后的整账户显式切换，不是请求级调度；不会检测限额后后台自动换号，也不会迁移正在运行的会话。
- 当前没有启动时自动更新或远程撤回机制。在线更新会将完整更新包读入内存后验签；严格下载字节上限是后续加固项。
- `tauri-action` 目前在同一受保护 job 中同时持有 updater 签名私钥和 GitHub `contents: write`；这是已知 P1 剩余风险，由 Environment 人工审批、main-only SHA、完整 action SHA 固定与 draft-only 限制。updater 私钥的独立离线恢复备份也必须在首次发布前完成。

## 已完成的安全验证

2026-08-27 完成一次全代码树安全扫描，确认的 12 项发现（9 medium、3 low）已全部修复，并由新的只读验证者逐项追踪为 `verified_fixed`。这是当时 unversioned working tree 的本机证据，不替代正式发布 commit 的重新扫描、签名与公证。

## 发布安全门

Community prerelease 必须确认测试、clippy、依赖审计、secret scan、SBOM/许可证、DMG 完整性与下载后 SHA-256，并确认没有 `latest.json` 或 updater bundle；发布文案必须披露 unsigned/unnotarized 状态。

签名稳定发布还必须确认 Developer ID 身份准确、Hardened Runtime 签名验证通过、notarytool 返回 accepted、stapler validate 通过、Tauri updater 签名与 `latest.json` 中绑定该版本 tarball asset ID 的 GitHub API URL 匹配、最终 artifact SHA-256 与发布记录一致。任何一步缺失都必须保持相应的 `unsigned community prerelease`、`signed`、`notarized`、`draft` 或 `unreleased` 状态，不能越级表述。
