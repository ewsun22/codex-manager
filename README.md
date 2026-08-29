# Codex Manager

Codex Manager 是一个独立运行、本地优先的开源 Codex 桌面管理器。它不接管 Codex、不做反向代理，主要用于观察本机 Codex 的模型交互元数据，并集中发现和安全编辑各项目的 `AGENTS.md`。

> 非官方项目，与 OpenAI 无隶属或背书关系。

源代码仓库：[github.com/ewsun22/codex-manager](https://github.com/ewsun22/codex-manager)

## 当前状态

当前源码版本是 **0.2.2 macOS 签名 beta 候选版**：它在 `v0.2.1` 首个 Developer ID 签名稳定版上优化大型活动库的启动与刷新路径，并增加每 5 秒自动显示最新已采集记录。`v0.2.0` 是不进入在线更新通道的 `Unsigned Community Build`；`v0.2.1` 已完成签名、公证、stapling、Gatekeeper、公开资产复验并成为可信 updater 起点。`v0.2.2` 必须重新通过同一 exact-SHA 发布门禁，并从已安装的 `v0.2.1` 完成真实更新、安装和重启验收后才能标记 accepted。

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
- 设置页提供用户主动的 GitHub Releases 更新检查；Rust 后端固定 endpoint 和公钥，复用已检查的更新对象，安装前显示 macOS 原生确认框并验证 Tauri 更新签名。

## 观测边界

Codex Manager 同时保留“来源”和“可信度”，不同来源不能混为一谈：

- rollout 适合历史、模型、effort、token 和 Codex 报告的耗时，但它是内部 JSONL，不是稳定公开 API；累计 token 增长也不能天然宣称为唯一网络请求。
- OTel `codex.api_request` 日志可提供请求级状态、耗时和安全化 endpoint 分类。本地只保存固定 event/provider/origin/route 枚举；已知价格模型保留标识，未知 model/thread/turn 仅保存不可逆的本地伪名指纹。原始 host/path、租户子域、用户名、密码、query 或 fragment 不落库。
- 当前 Codex 的 token OTel 日志位于 `codex.sse_event`，而聚合 metrics 通常没有 conversation id；本版不会猜测性地把这些记录拼到某个 API 请求上。
- 当前原生 Codex OTel 不提供可靠 wire response bytes，因此通常显示 `unavailable`。真实完整 URL、网络 TTFB、wire bytes 和供应商实际账单仍需要未来可选反代或服务端对账。
- API 等价价格不是 ChatGPT 套餐扣费，也不是中转站账单。未知模型或字段不完整时不估价。

详见 [能力矩阵](docs/capability-matrix.md) 与 [架构说明](docs/architecture.md)。

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

认证文件导入同样只接受该已验签 Codex 的隔离验证结果：Rust 原生选择器只读取当前用户拥有、权限不宽于 `0600`、单硬链接且不超过 128 KiB 的普通 JSON，并在读取前后复核文件身份与时间；WebView 不接收路径或内容，API Key 文件和非 ChatGPT OAuth 文件会被拒绝。实际 App Server PID 通过动态签名验证并恢复后，后端才使用官方 `chatgptAuthTokens` 登录请求在内存中提交 access token 和工作区 ID，再读取账户与额度；refresh token 不交给在线探测，也不创建临时 `auth.json`。验证后的档案秘密与内部元数据保存在 `cc.codex.manager.auth-profiles.v1` 应用专用 macOS Keychain 服务，不进入 SQLite。切换只支持 Codex 的 file 模式活动 `auth.json`：后端同时通过 App Server `config/read` 证明最终生效的 `cli_auth_credentials_store` 明确为 `file`，并要求文件属于当前用户、权限不宽于 `0600`、只有一个硬链接，再执行 Manager 实例间互斥、opaque revision、SHA/mtime/文件身份 CAS、同目录原子替换和切换后官方账户复核；如果官方流程刷新了活动凭据，管理器会重新读取新 bytes 和文件戳后再持久化或回滚。keyring/auto 模式会安全拒绝。官方 Codex 进程不遵守 Manager 的进程锁，因此删除提交前会再次复核活动文件，但用户仍应先结束正在运行的 Codex 任务。删除不是立即销毁：非活动且非最后一个可用档案会进入 30 天回收站，期间可以恢复，过期后才从应用专用 Keychain 清理。Codex CLI 与 IDE 扩展共享活动账户，因此切换影响之后读取凭据的新会话；本应用不会按请求或在后台自动换号，也不是反向代理。官方认证行为见 [Codex Authentication](https://developers.openai.com/codex/auth)，账户、配置与额度协议见 [Codex App Server](https://developers.openai.com/codex/app-server)。

更新检查也不会在启动时自动运行；只有用户在“设置 → 应用更新”中点击检查时才访问固定的 GitHub `latest.json`。

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

本机没有 Developer ID 时，`package-macos.sh` 只能生成 unsigned `.app`/`.dmg`。发布目录固定为不受 Vite `dist/` 清理影响的 `artifacts/release/`。2026-08-27 的历史本机产物是 `artifacts/release/Codex Manager_0.1.0_aarch64.dmg`，SHA-256 为 `a66968a63b3d246f8a1a3ee6e61800e01c2adf94f8dd0e7bdd4be145117fff74`；该值对应首次提交前的工作树，不能用于当前 `0.2.2` 签名候选。`.github/workflows/release.yml` 仅能手动从 `main` 当前 SHA 创建签名草稿 Release，缺少 Tauri 或 Apple 签名凭据时失败。签名、公证、更新清单与发布的状态边界见 [发布运行手册](docs/release.md)。

## 隐私与开源

- 默认只保存模型使用元数据、项目路径、checkpoint 和设置；不保存 Codex 消息正文、Authorization、Cookie、API Key、OAuth code、完整环境变量或请求 query。
- OAuth 页的 email、套餐、认证方式和额度仅保留在当前 UI 内存中；原始 App Server 响应不会落盘。用户明确导入的认证档案是例外：原始凭据只在原生后端的 zeroizing 内存中校验，验证后进入应用专用 macOS Keychain；WebView/SQLite/日志只接触不含 email、路径或 token 的档案 DTO。应用不导出/编辑凭据，不做代理式自动轮换。
- AGENTS 编辑功能需要在本地保存有限 revision，因此这些 revision 会包含用户主动编辑的 AGENTS 文本；每个文件最多保留 20 版。
- 数据默认不上传；唯一监听服务是用户显式开启的 loopback OTel receiver。
- 本项目采用 Apache-2.0。EasyCLIProxyAPI 只作为产品信息架构与行为研究参考；其 GUI 仓库未提供可确认的许可证，因此本项目采用基于 OpenAI 官方接口的 clean-room 实现，不复制其源码、样式、图标或品牌资产。

完整说明见 [PRIVACY.md](PRIVACY.md)、[SECURITY.md](SECURITY.md) 和 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
