# Codex Manager

Codex Manager 是一个独立运行、本地优先的开源 Codex 桌面管理器。它主要用于观察本机 Codex 的模型交互元数据、管理官方订阅账户与自定义 Responses 供应商，并集中发现和安全编辑各项目的 `AGENTS.md`。`v0.5.0` 新增的可选反代只监听 loopback、默认停止，不是系统代理或通用 API 网关。

> 非官方项目，与 OpenAI 无隶属或背书关系。

源代码仓库：[github.com/ewsun22/codex-manager](https://github.com/ewsun22/codex-manager)

## 当前状态

当前已定版的源码版本是 **0.5.0 签名稳定版候选源码**，包含 Codex 供应商与本地 Responses 网关切片；源码定版本身不代表已签名或 GitHub Release 已发布。`v0.5.0-beta.1` 已从 exact source `dd370070ae58c3e70d61e95f3ad7b38278080a50` 发布为隔离于稳定 updater 的 `Unsigned Community Build` Pre-release；`v0.2.1`、`v0.2.2`、`v0.3.0` 和 `v0.4.0` 已分别完成 Developer ID 签名、公证、stapling、Gatekeeper 与公开资产复验。`v0.5.0` 只有在最终 exact SHA 的单次成功 CI、受保护签名工作流、草稿回下载复验、人工 Publish 和公开发布后复验全部通过后，才能报告为 `published publicly verified`；真实供应商和升级安装完成前不能标记 `accepted`。

已实现的主要能力：

- 增量读取配置的 Codex home 下 `sessions` 与 `archived_sessions` rollout JSONL；活动 `sessions` 会先于归档积压处理，同一来源按最近修改时间优先。从同一个 no-follow 文件句柄取得身份、大小和内容，使用 UTF-8 byte checkpoint、完整行提交和定期 reconciliation。
- 将 session、turn、模型、provider、推理等级、token、缓存读、缓存写、reasoning output、首个可见输出和总耗时规范化到本机 SQLite。
- 以 session、ordinal、事件类型和累计 token 向量去重；重复快照不累加，非单调快照隔离为诊断。不同采集文件使用独立所有权 namespace，完全等价的 rollout 副本另通过 logical fingerprint 在查询时折叠，不会转移或删除原来投影。
- 总览、活动筛选、游标分页、详情抽屉、来源健康与缺失值 provenance；得不到的字段显示 `unavailable`，不会填 0。启动时先返回界面、最新活动和静态配置，需要逐次 model call 计价的总览汇总在后台补齐。活动页每 5 秒只检查小型采集水位，水位变化后才轻量重载当前页，不重算总数、不重跑全局 bootstrap，也不遮挡已有表格；“刷新本地数据”仍会显式执行一次有预算的 reconciliation。
- 版本化 API 等价价格目录，按一次 model call 分桶计算普通输入、缓存读、缓存写和输出；reasoning output 不重复计费。
- 用户主动开启的 TLS loopback OTLP/HTTP receiver：首次启用时随机分配空闲端口，后续复用应用私有配置。客户端必须通过本机 CA 校验服务端身份，receiver 再用专用 header 认证调用方；认证发生在 body 解码前，请求体上限 128 KiB，空闲连接 10 秒超时，日志 body 从不读取或持久化。
- 从已观测 cwd 与用户授权根目录发现项目、Git root 和 worktree，解析 Codex 当前的全局到 cwd `AGENTS` 有效链。
- “项目与 AGENTS”会优先展示顶级 Codex home `AGENTS.md`；项目列表以绿点表示项目根目录存在普通 `AGENTS.md`，以黄点表示缺失，并保留递归发现数量供核对。
- 在授权根目录内创建项目级 `AGENTS.md`，并以 canonical path、逐段 no-follow、SHA/mtime 冲突检查、同目录原子交换和本地 revision 完成保存与恢复。
- 探测 Desktop 内置或 PATH 中的 Codex CLI，并生成 App Server JSON Schema 指纹；不附着 Codex Desktop 的私有 stdio。
- OAuth 页面由用户点击启动官方 `codex login` 浏览器流程，并用短生命周期的官方 App Server 读取账户、套餐与 ChatGPT Codex 额度窗口。macOS beta 还支持通过原生文件选择器导入 ChatGPT OAuth `auth.json`、把多个档案秘密保存在应用专用 Keychain、软删除/恢复，以及由用户明确触发的“设为当前”或“轮换到下一个”。token、文件路径、原始 JSON、授权 URL 与回调 URL不进入 WebView、SQLite 或应用日志。
- `v0.5.0` 提供 Codex Responses-compatible 供应商和显式启停的本地网关；未发布界面进一步把这一工作流整理为“Codex 配置管理”：供应商改用紧凑卡片与新增/编辑弹窗，页面就近展示网关状态、配置预览入口和官方订阅边界。供应商元数据进入 SQLite，API Key 与随机本机 bearer 只进入应用专用 Keychain/原生运行时；普通 DTO 只返回 `hasApiKey`。网关固定监听 `127.0.0.1`，只支持 `/v1/models`、`/v1/responses`、`/v1/responses/compact` 和受保护的本机辅助端点，禁用 redirect 和系统代理，不保存请求/响应 body、header 或 query。界面中的“应用”只启动所选供应商的 loopback 网关，不会自动改写 `config.toml`；含 bearer 的配置片段仍只在用户通过原生确认后临时显示。
- 设置页提供 GitHub Releases 更新检查：桌面版启动后按需检查，并默认每 12 小时自动检查一次；检查间隔可在设置中配置为 1–168 小时。Rust 后端固定 endpoint 和公钥，只持久化最近检查尝试时间，以及最近成功检查的当前/可用版本、发布日期和截断后的 notes；发现新版本时设置侧栏与应用更新卡片都会提醒。安装前显示 macOS 原生确认框并验证 Tauri 更新签名，下载、安装和重启仍需用户显式操作。

## 观测边界

Codex Manager 同时保留“来源”和“可信度”，不同来源不能混为一谈：

- rollout 适合历史、模型、effort、token 和 Codex 报告的耗时，但它是内部 JSONL，不是稳定公开 API；累计 token 增长也不能天然宣称为唯一网络请求。
- OTel `codex.api_request` 日志可提供请求级状态、耗时和安全化 endpoint 分类。本地只保存固定 event/provider/origin/route 枚举；已知价格模型保留标识，未知 model/thread/turn 仅保存不可逆的本地伪名指纹。原始 host/path、租户子域、用户名、密码、query 或 fragment 不落库。
- 当前 Codex 的 token OTel 日志位于 `codex.sse_event`，而聚合 metrics 通常没有 conversation id；本版不会猜测性地把这些记录拼到某个 API 请求上。
- 当前原生 Codex OTel 不提供可靠 wire response bytes，因此通常显示 `unavailable`。`v0.5.0` 网关首版只提供运行状态和有界请求/失败/并发计数，尚不把 TTFB、wire bytes 或供应商账单写入活动数据库；未经过网关的官方直连仍显示 `unavailable`。
- API 等价价格不是 ChatGPT 套餐扣费，也不是中转站账单。未知模型或字段不完整时不估价。

详见 [能力矩阵](docs/capability-matrix.md) 与 [架构说明](docs/architecture.md)。

## 活动记录与估算口径

活动页同时提供“任务汇总”和“模型交互”两种粒度：任务汇总代表一个 `turn`，模型交互代表已观测到的单次模型/token 事件。交互事件会显示所属任务状态；当来源没有终态时显示 `unobserved`，不会把仍在采集、已完成的后续事件或缺少终态的记录误报为 `running` 或 `success`。重复的累计 `token_count` 快照按 `session`、`ordinal`、事件类型和累计向量去重，不会直接累加快照。

输入、缓存读写和输出数值旁的来源说明只在详情或说明区域显示，含义是数据来源与推导方式，不是“请求成功”的保证。rollout 不提供可靠的调用级首个输出和总耗时时显示 `unavailable`；可关联的任务级时间才会展示。列表顶部和底部均提供上一页/下一页切换。

“API 等价估算”按去重后的 token 分项计价，展示已覆盖调用数与 token 数；未知模型或缺字段的部分保持未覆盖，不按零值或默认模型补算。它不是 ChatGPT 套餐扣费，也不是中转账单。

“项目与 AGENTS”中的最近更新时间取该项目最后一次已观测对话时间，而不是重新发现时间；从未观测到对话的项目明确显示 `unavailable`。

## 本地开发

首发目标为 macOS 13+。开发环境固定 Node.js 24 与 Rust 1.98.0：

```bash
npm ci
npm run typecheck
npm test
cargo test --workspace
npm run tauri:dev
```

首次启动会尝试发现现有 `~/.codex`。要编辑项目文件，仍需在“设置”中明确保存授权项目根目录；只观察到项目并不自动授予写权限。
OAuth 页面读取状态时会启动一次有超时和响应上限的本机 `codex app-server`；只有用户点击“开始登录/重新登录”才运行 `codex login`。macOS 首次使用时会把固定 ChatGPT bundle 中的 Codex 复制到随机 `0700` 临时目录，再用系统 `codesign` 验证 identifier `codex` 与 OpenAI Team ID `2DC432GLL2`；该稳定副本在应用生命周期内复用，但每次启动前都会复验路径。App Server 通过 `POSIX_SPAWN_START_SUSPENDED` 暂停启动，后端以 `+PID` 动态验证实际进程后才恢复执行；PATH、环境覆盖和验签后路径替换不会跨越认证边界。当前非 macOS 构建显示 OAuth `unavailable`。登录进程由后端独立监督并在 10 分钟内退出或强制回收。

认证文件导入同样只接受该已验签 Codex 的隔离验证结果：Rust 原生选择器只读取当前用户拥有、权限不宽于 `0600`、单硬链接且不超过 128 KiB 的普通 JSON，并在读取前后复核文件身份与时间；WebView 不接收路径或内容，API Key 文件和非 ChatGPT OAuth 文件会被拒绝。实际 App Server PID 通过动态签名验证并恢复后，后端才使用官方 `chatgptAuthTokens` 登录请求在内存中提交 access token 和工作区 ID，再读取账户与额度；refresh token 不交给在线探测，也不创建临时 `auth.json`。验证后的档案秘密与内部元数据保存在 `cc.codex.manager.auth-profiles.v1` 应用专用 macOS Keychain 服务，不进入 SQLite。

切换只支持 Codex 的 file 模式活动 `auth.json`：后端同时通过 App Server `config/read` 证明最终生效的 `cli_auth_credentials_store` 明确为 `file`，并要求文件属于当前用户、权限不宽于 `0600`、只有一个硬链接，再执行 Manager 实例间互斥、opaque revision、SHA/mtime/文件身份 CAS、同目录原子替换和切换后官方账户复核；如果官方流程刷新了活动凭据，管理器会重新读取新 bytes 和文件戳后再持久化或回滚。keyring/auto 模式会安全拒绝。官方 Codex 进程不遵守 Manager 的进程锁，因此删除提交前会再次复核活动文件，但用户仍应先结束正在运行的 Codex 任务。删除不是立即销毁：非活动且非最后一个可用档案会进入 30 天回收站，期间可以恢复，过期后才从应用专用 Keychain 清理。Codex CLI 与 IDE 扩展共享活动账户，因此切换影响之后读取凭据的新会话；本应用不会按请求或在后台自动换号。官方 OAuth 凭据不会进入 `v0.5.0` 的 API-key 网关。官方认证行为见 [Codex Authentication](https://developers.openai.com/codex/auth)，账户、配置与额度协议见 [Codex App Server](https://developers.openai.com/codex/app-server)。

`v0.5.0` 网关只在用户选择一个已保存的 API-key 供应商并点击启动后监听。远程 Base URL 必须是解析到公网地址的 HTTPS origin，`http` 仅允许明确的 `localhost`/loopback 开发上游；userinfo、query、fragment、私网/link-local/unspecified/multicast 地址和 30x redirect 会被拒绝。网关不会安装 CA、注册系统代理、常驻后台或自动接管 Codex。要使用它，用户需在原生确认后自行将生成的 provider 片段合并到 Codex 配置；停止服务前应先恢复直连配置。

桌面版启动后会按需检查更新，并按设置的间隔（默认 12 小时，允许 1–168 小时）再次检查；仅访问后端固定的 GitHub `latest.json`。成功或失败都从最近一次检查尝试起遵守该间隔；自动检查失败不会打断使用，也不会覆盖上一次成功检查状态。浏览器 demo 不自动联网。检查状态只保留最近尝试时间与受限展示元数据，不保存 URL、签名对象、下载句柄、错误详情或凭据；下载、签名校验、安装和重启仍要求用户显式操作，并在同进程重新检查、版本核对和 macOS 原生确认后继续。

浏览器预览使用人工构造的脱敏 demo 数据：

```bash
npm run dev
```

## 可选 OTel 增强

先在 Codex Manager 设置中启用本机 receiver，再点击“显示 OTel 配置”。应用首次从 IPv4 loopback 随机分配空闲端口，将端口与 localhost TLS 身份保存在应用私有目录后复用；如果该端口被其他进程占用，receiver 会安全失败，不会无声切换到未配置的端口。它不会改写 Codex 配置。界面会按当前 endpoint、CA 路径和专用 token 生成完整片段，结构如下：

```toml
[otel]
log_user_prompt = false
exporter = { otlp-http = { endpoint = "https://localhost:<persisted-random-port>/v1/logs", protocol = "binary", headers = { "x-codex-manager-token" = "<generated-token>" }, tls = { ca-certificate = "/absolute/app-data/localhost-cert.pem" } } }
metrics_exporter = { otlp-http = { endpoint = "https://localhost:<persisted-random-port>/v1/metrics", protocol = "binary", headers = { "x-codex-manager-token" = "<generated-token>" }, tls = { ca-certificate = "/absolute/app-data/localhost-cert.pem" } } }
```

`log_user_prompt` 必须保持 `false` 才符合本项目默认隐私边界。官方配置字段以 [Codex Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference) 为准。

## 检查与打包

```bash
npm run typecheck
npm test
npm run build
./scripts/check-version-sync.sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./scripts/audit-dependencies.sh
./scripts/scan-secrets.sh
./scripts/package-macos.sh
./scripts/sbom.sh
./scripts/licenses.sh
./scripts/verify-release.sh unsigned
```

本机没有 Developer ID 时，`package-macos.sh` 只能生成 unsigned `.app`/`.dmg`。发布目录固定为不受 Vite `dist/` 清理影响的 `artifacts/release/`。2026-08-27 的历史本机产物是 `artifacts/release/Codex Manager_0.1.0_aarch64.dmg`，SHA-256 为 `a66968a63b3d246f8a1a3ee6e61800e01c2adf94f8dd0e7bdd4be145117fff74`；该值对应首次提交前的工作树，不能用于当前版本。已发布的 `v0.5.0-beta.1` community Pre-release 仅使用当时最终 `main` exact SHA 的成功 CI unsigned artifact；`v0.5.0` 继续只允许通过 `.github/workflows/release.yml` 的受保护签名稳定通道发布。签名、公证、更新清单与发布的状态边界见 [发布运行手册](docs/release.md)。

## 隐私与开源

- 默认只保存模型使用元数据、项目路径、checkpoint、供应商非秘密元数据和设置；不保存 Codex 消息正文、Authorization、Cookie、OAuth code、完整环境变量或请求 query。用户提供的供应商 API Key 与本机网关 bearer 只进入应用专用 Keychain/原生运行时，不进入 SQLite、普通 WebView DTO 或日志。
- OAuth 页的 email、套餐、认证方式和额度仅保留在当前 UI 内存中；原始 App Server 响应不会落盘。用户明确导入的认证档案是例外：原始凭据只在原生后端的 zeroizing 内存中校验，验证后进入应用专用 macOS Keychain；WebView/SQLite/日志只接触不含 email、路径或 token 的档案 DTO。应用不导出/编辑凭据，不做代理式自动轮换。
- AGENTS 编辑功能需要在本地保存有限 revision，因此这些 revision 会包含用户主动编辑的 AGENTS 文本；每个文件最多保留 20 版。
- 数据默认不上传；只有用户显式开启的 loopback OTel receiver 和 `v0.5.0` Codex Responses 网关会监听本机端口。网关启动后会把用户主动发送给它的请求转发到所选供应商，因此该供应商按其自身隐私条款接收内容。
- 本项目采用 Apache-2.0。AI Toolbox 的 MIT 仓库仅以固定 commit `3f2faef5b7a453169d4325d89fab290154233723` 作为产品行为和失败模式参考；本功能基于 OpenAI 官方 Responses/config/App Server 契约 clean-room 实现，不复制其源码、样式、图标、品牌或来自 AxonHub 的 LGPL transformer fixture。EasyCLIProxyAPI 仍只作为产品信息架构参考；未确认许可证的 GUI 代码不进入本仓库。

完整说明见 [PRIVACY.md](PRIVACY.md)、[SECURITY.md](SECURITY.md) 和 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
