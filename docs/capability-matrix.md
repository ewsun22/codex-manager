# 能力矩阵

| 能力 | Rollout adapter | OTel logs | OTel metrics | App Server | 当前 UI 结论 |
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
| 应用在线更新 | 不负责 | 不负责 | 不负责 | 不负责 | 用户手动触发；GitHub Releases + Tauri 签名 + macOS 原生确认 |

## 活动记录字段口径（当前版本）

| 展示层级 | 一行代表 | 状态 | 首个输出/总耗时 | token 与来源说明 |
|---|---|---|---|---|
| 任务汇总 | 一个 `turn` | `running`、`success`、`failure` 或 `unobserved` | 仅在 turn 级来源可靠时展示 | 聚合去重后的累计向量 |
| 模型交互 | 一个已观测的 model/token 事件 | 继承所属任务状态并明确“事件已记录” | rollout 无可靠调用级值时为 `unavailable` | 数值旁不重复显示标签，详情提供 source/provenance |

`unobserved` 表示来源没有可靠终态，不等同成功、失败或仍在运行。`unavailable` 表示该来源没有提供可验证字段，不是零值。API 等价价格可以部分覆盖：未识别模型或字段不完整的调用计入未覆盖数量，不参与金额估算。

## 不承诺的能力

- 默认“活动记录总数”是 canonical `turn` 任务数；“模型交互”视图是 canonical model-call 与已观测 OTel request 行数。两者都不是供应商账单请求数。
- rollout JSONL 是私有兼容源；版本变化可能造成 source health 降级。
- 没有反代时，不能精确取得完整请求 URL、真实网络 TTFB、wire bytes 或供应商账单。
- App Server 不附着 Codex Desktop 内部进程，也不读取历史。除 Schema probe 外，OAuth 页只创建短生命周期进程读取官方账户与额度 DTO。
- 不提供认证文件导出/编辑、请求级自动轮换或 token 代理；显式“轮换到下一个”是共享活动账户切换，不迁移正在运行的会话。不把 ChatGPT Codex 额度宣称为 OpenAI Platform API 费率、API 等价成本或供应商账单。
- OAuth 认证边界当前只在 macOS 启用，只执行从固定 ChatGPT bundle 复制到随机私有目录并在复制后通过 OpenAI identifier/Team ID 验证的 Codex；其他平台显示 `unavailable`，等待等价的发布者身份验证。
- 导入依赖 App Server 当前的 `chatgptAuthTokens` 协议，该协议变更时会 fail closed。导入时必须有可用 access token；为避免触发 refresh token 轮换，不在导入探测中单独验证 refresh token。
- OTel metrics 是聚合数据；当前 Codex metrics 缺少稳定的逐 conversation 关联时，本应用不会把 histogram/count 展开为虚假的单次请求。
