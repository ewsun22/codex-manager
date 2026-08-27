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
| API 等价价格 | token + 内置目录估算 | token 完整时可估算 | 不按聚合指标估价 | unavailable | 不是套餐或中转实际账单 |
| 项目发现 | cwd observed | 不负责 | 不负责 | 不负责 | 已观测 cwd + 用户授权扫描根 |
| AGENTS 有效链 | 文件系统 adapter | 不负责 | 不负责 | 文档规则对齐 | 全局到选定 cwd |
| AGENTS 安全写入 / revision | 不适用 | 不适用 | 不适用 | 不适用 | 仅显式授权根目录 |
| 应用在线更新 | 不负责 | 不负责 | 不负责 | 不负责 | 用户手动触发；GitHub Releases + Tauri 签名 + macOS 原生确认 |

## 不承诺的能力

- “活动记录总数”是本地来源的观测行数，不是经过跨来源关联后的唯一账单请求数。
- rollout JSONL 是私有兼容源；版本变化可能造成 source health 降级。
- 没有反代时，不能精确取得完整请求 URL、真实网络 TTFB、wire bytes 或供应商账单。
- App Server 当前只做命令可用性与 Schema SHA-256 探测，没有附着 Codex Desktop 内部进程，也没有用它读取历史。
- OTel metrics 是聚合数据；当前 Codex metrics 缺少稳定的逐 conversation 关联时，本应用不会把 histogram/count 展开为虚假的单次请求。
