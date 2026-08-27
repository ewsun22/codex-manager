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
```

## 采集层

### Rollout adapter

- 只扫描用户配置 Codex home 的 `sessions` 和 `archived_sessions`。
- canonical path 必须仍位于允许目录；拒绝符号链接和非普通文件。
- 以设备/文件身份、UTF-8 byte offset 和 parser resume state 增量读取。
- 单行上限 4 MiB；EOF 未完成行不提交，但仍计入读取预算；超长未结束行不从中间 checkpoint，避免把攻击者选定的后缀当成新事件。
- 文件截断或身份变化时，仅重建该来源的可重建投影。
- watcher 只使用一个合并信号槽，满时丢弃重复唤醒，并每 60 秒 reconciliation。每轮同时限制遍历数、字节、事件和时间，达到预算时显示 `partial/degraded`。

JSONL 是兼容适配源，不被描述为稳定公开 API。新的事件源必须实现 adapter，UI 不直接理解私有 JSON 结构。

### OTel adapter

- 用户显式启用后只绑定 `127.0.0.1` 的首次随机、后续持久化端口；自签 localhost 证书提供服务器身份，每个请求还必须在 body 解码前通过定时比较的专用 header 认证。持久化端口被占用时 fail closed，不自动切换到 Codex 未配置的端口。
- 支持 OTLP/HTTP protobuf 与 JSON；请求体上限 128 KiB，最多 16 条并发 TLS 连接、4 个并发请求，连接每 10 秒无读写进展即释放；另限制频率、3 秒处理时间和 resource/scope/event/attribute/data-point 数量。一批最多 256 条，单事务落库，超配额整批拒绝。
- 日志 body 从不读取。只物化固定 allowlist：未知 event/provider 折叠为 `other`，已知价格模型保留标识，未知 model/thread/turn 仅持久化带类型的 SHA-256 截断伪名，不保存未受控的自由文本。
- endpoint 在落盘前只保留固定 provider/origin 分类和已知 route 枚举；原始 host、租户子域、path、username、password、query 与 fragment 均不保留。
- 活动列表只把 `logs/codex.api_request` 作为请求级记录。聚合 metrics 不展开成虚假的逐请求行；`codex.sse_event` token 也不在缺少可靠关联键时猜测合并。

### App Server capability

应用从 Desktop bundle、显式环境变量或 PATH 寻找受信任的 Codex 可执行文件，使用有超时、无 stdin 的子进程在应用私有临时目录生成 JSON Schema，并对有数量、单文件与总大小上限的产物做 SHA-256 指纹。它只证明本机 CLI 支持对应命令，不代表已经连接或订阅 Codex Desktop 内部 app-server。

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

发现文件不等于写授权。写入必须同时通过已知项目、显式授权根目录、canonical containment、文件名 allowlist、逐段 no-follow、安全文件身份、expected SHA/mtime 和最大大小检查。Unix 上从 `/` 开始打开根目录链，每个组件均通过 `openat(O_DIRECTORY | O_NOFOLLOW)` 验证；随后在同一目录创建临时文件、fsync，并在 atomic swap 后以同一安全路径重验 CAS，冲突时原子回滚，最后提交 revision。应用不会执行或信任 AGENTS 内容中的命令。

## 前端与 IPC

React 只使用 18 个显式 Tauri commands；其中 OTel 凭据只在用户点击显示配置时返回前端。Tauri capability 清单不授予通用 shell 或任意文件系统 API，WebView 使用严格 CSP。浏览器开发模式使用人工构造的 demo adapter，不读取本机 Codex 数据。

## 跨平台边界

跨平台：event normalization、SQLite schema、价格、项目模型、AGENTS precedence、DTO 与 React UI。

平台适配：可执行文件发现、稳定文件身份、文件 no-follow/原子替换、应用数据目录、打包签名。macOS 使用 `openat`/`O_NOFOLLOW` 路径；Windows 当前只有可编译的 portable fallback，必须在 Windows beta 前补齐等价的 reparse-point/原子替换安全验证和安装包测试。

## 官方依据

- [Codex Advanced Configuration](https://learn.chatgpt.com/docs/config-file/config-advanced)
- [Codex Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [Custom instructions with AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- [OpenAI API Pricing](https://learn.chatgpt.com/docs/pricing)
