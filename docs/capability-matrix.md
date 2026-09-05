# 能力矩阵

| 能力 | Rollout adapter | OTel logs | OTel metrics | CLI Schema / App Server | 当前 UI 结论 |
|---|---|---|---|---|---|
| 历史 session/turn/item 顺序 | observed | 不负责 | 不负责 | 仅 schema probe | rollout 元数据可用 |
| 消息正文 | 禁止持久化 | body 不读取 | 不适用 | 未连接 | 不提供 |
| 模型 / provider / effort | observed | model；部分事件含 provider/effort | model 为聚合标签 | 未连接 | 按字段显示来源；缺失为 unavailable |
| input/output/cache-read/cache-write/reasoning token | observed，累计向量去重 | `sse_event` 可接收，但不猜测关联 API request | 聚合 histogram，无可靠逐会话关联 | 未连接 | 请求行优先使用 rollout；OTel 未关联值不拼接 |
| turn 总耗时 / first visible output | codex-reported | API/SSE 各自 duration | 聚合 histogram | 未连接 | 不等同代理层 TTFB |
| HTTP status / success / retry | unavailable | `codex.api_request` observed | 仅聚合标签 | unavailable | OTel 请求行可用 |
| 请求 endpoint | unavailable | 观测后映射为固定 provider/origin 分类与已知 route | unavailable | unavailable | OTel 请求行可用；不保存完整原始 URL |
| response bytes | unavailable | 仅当 producer 提供 body-size 字段；当前原生 Codex 通常不提供 | unavailable | unavailable | 通常 unavailable |
| API 等价价格 | token + 内置目录估算 | 按已识别模型的完整 token 分项估算 | 不按聚合指标估价 | unavailable | 显示已覆盖/未覆盖；不是套餐或中转实际账单 |
| 项目发现 | cwd observed | 不负责 | 不负责 | 不负责 | 已观测 cwd + 用户授权扫描根 |
| AGENTS 有效链 | 文件系统 adapter | 不负责 | 不负责 | 文档规则对齐 | 全局到选定 cwd |
| AGENTS 安全写入 / revision | 不适用 | 不适用 | 不适用 | 不适用 | 仅显式授权根目录 |
| Codex 登录状态 | 不负责 | 不负责 | 不负责 | `account/read`，短生命周期只读 | 仅白名单摘要；不展示 token/授权 URL |
| ChatGPT Codex 额度 | 不负责 | 不负责 | 不负责 | `account/rateLimits/read` | 官方窗口；不是 API 费率或本地估算 |
| OAuth 登录启动 | 不负责 | 不负责 | 不负责 | 官方 `codex login` | 用户点击后由系统浏览器和 Codex 完成 |
| 认证文件导入 | 不负责 | 不负责 | 不负责 | 暂停进程动态验签后，以 `chatgptAuthTokens` 外部认证 + `account/read` + `account/rateLimits/read` 验证 access token | 原生选择器；refresh token 不交给在线探测；秘密进入应用专用 Keychain |
| 多账户切换 | 不负责 | 不负责 | 不负责 | `config/read` 证明 file 模式，切换前后 `account/read` 复核 | 用户确认后 CAS + 原子替换；keyring/auto 拒绝 |
| 认证档案删除/恢复 | 不负责 | 不负责 | 不负责 | 不负责 | 活动/最后档案不可删；软删除 30 天可恢复 |
| 应用在线更新 | 不负责 | 不负责 | 不负责 | 不负责 | 桌面端启动后按需检查，默认每 12 小时自动检查（可配置 1–168 小时）；固定 GitHub Releases `latest.json` + Tauri 签名 + macOS 原生确认；新版本在侧栏和应用更新卡片提醒 |
| CLI Schema 兼容性 | 不负责 | 不负责 | 不负责 | 短生命周期 Schema probe；最近版本、状态和指纹持久化 | 非采集源；不附着 Desktop 内部 App Server，不影响 rollout 主采集 |
| 官方订阅缓存 | 不负责 | 不负责 | 不负责 | 账户/套餐/额度读取后白名单摘要 | cache-first；仅 macOS Keychain，作为白名单 DTO供当前页面读取；cache miss/手动刷新才解析可信 CLI；超过 6 小时或刷新失败显式 stale |

## 双控制面与 CLIProxyAPI 独立内核

| 能力 | 数据/运行来源 | 当前结论 |
|---|---|---|
| OpenAI-compatible 供应商 | SQLite 非秘密元数据 + 应用专用 Keychain API Key | macOS；名称、HTTP(S) Base URL、模型和 reasoning effort 可管理；普通 DTO 只有 `hasApiKey`；地址由 Codex 直连，保存时不做 DNS/私网门禁 |
| Codex 配置控制面 | Codex `config.toml` + SQLite 非秘密 profile | 独立页面管理官方直连、本地 CLIProxyAPI、已保存外部 Responses provider；支持预览、CAS/原子事务应用与恢复；仅维护 `model`、`model_provider` 和一个 provider table |
| 本地代理控制面 | 预编译 CLIProxyAPI + 独立 OAuth 凭据域 | 独立页面/首页开关管理内核、监听状态、接入地址与 OAuth 凭据池；不与官方 OAuth 档案共用 auth-dir 或 Keychain service |
| 内核来源与版本 | `router-for-me/CLIProxyAPI` 官方 GitHub tag 资产 | 桌面主体与内核独立版本；用户显式确认/安装；内测固定审核基线 `v7.2.145`，不追随 `latest`；本机不编译 Go |
| 下载与安装 | Rust supervisor + 私有 staging/事务标记 | 内置精确资产名与 SHA-256；下载/解压有上限，拒绝穿越和链接；记录已解压二进制摘要，状态/启动前复核，不匹配时可显式切回审核基线 |
| 本地监听 | CLIProxyAPI 预编译 sidecar | 默认停止；生成配置固定 IPv4 loopback 与用户选定端口；PID + `/healthz` 检查；不会安装系统代理或后台 helper |
| 客户端认证 | 本机 loopback API key | 仅显示非秘密接入状态和 URL；Codex 配置通过 `auth.command` 读取原生 helper，配置只存 app binary path 与 allowlisted opaque secret ref，不保存 bearer/API Key |
| 上游转发 | CLIProxyAPI OAuth provider pool | 当前只通过 CLIProxyAPI OAuth 凭据池转发；核心提供多协议 client endpoint，但 Claude/Gemini 是否可用取决于凭据和内核，不宣称已做真实 E2E |
| 外部 API-key 配置 | Codex `model_providers` + Keychain | 外部 API-key 供应商由 Codex 配置直接使用，不经过 CLIProxyAPI；普通 DTO 只返回 `hasApiKey` |
| CLIProxyAPI OAuth 凭据 | Keychain + 私有 auth-dir | OAuth token 静态保留在独立 Keychain；运行时进入 `0700` 目录/`0600` 文件，停止时删除；不进入 SQLite、普通 DTO、日志或备份 |
| CLIProxyAPI OAuth 凭据池 | 原生文件选择器 + `cc.codex.manager.cliproxy-auth.v1` Keychain | 显式导入 `codex`/`claude`/`antigravity`/`kimi`/`xai` 当前理解格式；DTO 不含账户、路径、token、fingerprint；identity 只用于本地防串号，不是联网验真 |
| OAuth 运行时投影 | 随机私有 auth-dir + checkpoint | 启动时投影到 `0700`/`0600` 文件，允许内核原地 refresh；正常停止先做 provider/identity/CAS checkpoint 再清理；pending 崩溃证据 fail closed，确认无 orphan 后才恢复 |
| 日志与控制面 | 生成配置 fail closed | `commercial-mode:true`，request/debug/file log/usage stats 关闭；Management API、OAuth callback、动态插件不向 WebView 开放 |
| 请求观测 | CLIProxyAPI usage stats 禁用 | 请求数、失败数和 in-flight 均为 `unavailable`，不以 0 伪装；不保存 body、header、query、原始上游错误、TTFB 或 wire bytes |
| Codex 配置应用 | 原生 `auth.command` + 私有 journal | 只允许 `local-proxy-client` 与 `external-provider:<uuid>` opaque ref；接管前备份存入私有 journal，文件漂移拒绝覆盖；verified 仅表示写后文件复核，不表示 Codex 已成功请求上游 |
| 官方订阅 | 既有受信任 Codex CLI/App Server/Keychain 链路 | 官方账户、套餐、额度、显式切换继续独立；官方 OAuth token 永不进入 CLIProxyAPI |

## 活动记录字段口径（当前版本）

| 展示层级 | 一行代表 | 状态 | 首个输出/总耗时 | token 与来源说明 |
|---|---|---|---|---|
| 任务汇总 | 一个 `turn` | `running`、`success`、`failure` 或 `unobserved` | 仅在 turn 级来源可靠时展示 | 聚合去重后的累计向量 |
| 模型交互 | 一个已观测的 model/token 事件 | 继承所属任务状态并明确“事件已记录” | rollout 无可靠调用级值时为 `unavailable` | 数值旁不重复显示标签，详情提供 source/provenance |

`unobserved` 表示来源没有可靠终态，不等同成功、失败或仍在运行。`unavailable` 表示该来源没有提供可验证字段，不是零值。API 等价价格可以部分覆盖：未识别模型或字段不完整的调用计入未覆盖数量，不参与金额估算。

v0.6.6 进一步约束任务完整性：任务出现异常 token 快照后，任务用量保持 `unavailable`，此前有效调用仍可作为部分观测进入模型交互和估价。缺失的调用模型、provider、effort 在活动和计价中采用一致的任务回退规则，不覆盖调用显式值；整批与逐批读取的估价输入保持一致。模型与推理级别筛选候选覆盖所选视图的全部已采集记录，不受当前页限制。

保留期适用于首次导入、重建与周期维护；过期且已观测终态的任务及其关联元数据事务化删除，并更新活动 revision。未观测终态或没有可用时间戳的任务暂予保留。AGENTS 编辑增加未保存保护与迟到读取隔离；本地代理状态在页面可见时刷新，但进程存活状态仍不代表真实上游请求验收。

## 不承诺的能力

- 默认“活动记录总数”是 canonical `turn` 任务数；“模型交互”视图是 canonical model-call 与已观测 OTel request 行数。两者都不是供应商账单请求数。
- rollout JSONL 是私有兼容源；版本变化可能造成 source health 降级。
- CLIProxyAPI 请求不能精确取得完整请求 URL、真实网络 TTFB、wire bytes 或供应商账单；usage statistics 固定关闭，因此状态 DTO 中请求计数为 `unavailable`。
- App Server 不附着 Codex Desktop 内部进程，也不读取历史。除 Schema probe 外，OAuth 页只创建短生命周期进程读取官方账户与额度 DTO。
- 不提供官方 OAuth token 导出/编辑或跨信任域共享；CLIProxyAPI OAuth 仅支持原生导入、启停、启用/禁用、删除/恢复和受控运行时 checkpoint，不把 ChatGPT Codex 额度宣称为 OpenAI Platform API 费率、API 等价成本或供应商账单。
- OAuth 认证边界当前只在 macOS 启用，只执行从固定 ChatGPT bundle 复制到随机私有目录并在复制后通过 OpenAI identifier/Team ID 验证的 Codex；其他平台显示 `unavailable`，等待等价的发布者身份验证。
- 导入依赖 App Server 当前的 `chatgptAuthTokens` 协议，该协议变更时会 fail closed。导入时必须有可用 access token；为避免触发 refresh token 轮换，不在导入探测中单独验证 refresh token。
- OTel metrics 是聚合数据；当前 Codex metrics 缺少稳定的逐 conversation 关联时，本应用不会把 histogram/count 展开为虚假的单次请求。
- 浏览器 demo 不自动联网；自动更新检查仅属于桌面后端。
- CLIProxyAPI 上游 release 缺少 detached signature、Developer ID、公证、SBOM 与可复现 provenance；内置 SHA-256 只用于拒绝与已审核资产不同的下载字节。内测固定 `v7.2.145` 精确摘要；隔离空池观测已完成，真实 OAuth E2E 仍待完成，不把上游供应链增强项作为内测门禁。

## 界面语言

| 能力 | 当前状态 | 口径 |
| --- | --- | --- |
| 中英文界面 | implemented | 支持简体中文和 English；首次按设备语言选择，`zh-*` 为中文，其他语言为 English |
| 手动切换 | implemented | 顶部切换器即时生效，偏好只保存在本地 WebView |
| 数据格式化 | implemented | 数字、日期、金额随 locale 格式化；用户数据和外部 notes 不翻译 |

## v0.6.6 调度和验收边界

变更文件优先且不依赖全目录枚举；历史发现与单文件续读保留跨轮进度，队列溢出以周期 reconciliation 补偿。冷启动历史顺序不再承诺全局 mtime 优先，活动展示仍按观测时间查询。规模基准只代表人工暖缓存目录发现和增量 checkpoint，不代表真实会话全量解析或 UI 延迟。

AGENTS 创建、恢复、未保存离开与窗口关闭采用应用内异步确认，在 macOS WebView 中可实际选择取消或确认。debug-only `desktop-qa` 使用真实 SQLite/Tauri IPC 与人工项目验证这些行为，但没有真实凭据、代理或 updater 能力；不能据此宣称 OAuth 或安装升级完成。
