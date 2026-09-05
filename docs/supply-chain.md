# 供应链与可复现发布

- `package-lock.json` 与 `Cargo.lock` 必须进入版本控制；CI 使用 `npm ci`。
- GitHub Actions 第三方 action 以完整 commit SHA 固定，并在行尾注释 tag。
- 手动 release workflow 只接受 `main` 当前 SHA，使用有 required reviewer 和 main-only 规则的 `release` Environment；Tauri/Apple 私密凭据只存在 Environment secrets。缺少任一凭据必须 fail closed。
- 发布前必须运行 `npm audit --json` 和固定版本 `cargo-audit`；缺少审计工具、报告不可解析或 npm 任意等级 vulnerability 总数非零都直接失败，不允许以 `--audit-level` 阈值跳过低等级结果。
- `scripts/scan-secrets.sh` 使用仓库内 Node scanner，不依赖 runner 是否预装 `rg`。它对 Git 候选文件逐字节扫描高风险凭据格式和禁止证书后缀，覆盖 encrypted PKCS8、YAML block secret、RFC Bearer 字符与尾随空白伪装后缀，只豁免精确 `${{ secrets.NAME }}` 引用；只输出 JSON 转义后的文件名。退出码 `0/1/2` 分别表示 clean/finding/scanner error，任何未完整扫描都 fail closed。
- `scripts/sbom.sh` 生成 npm 应用 SBOM，并为 `codex-core`、`codex-storage`、`codex-manager-desktop` 三个 Rust package 分别生成 CycloneDX JSON；`scripts/licenses.sh` 生成许可证清单。
- `scripts/verify-release.sh` 为每个构建目录生成 SHA-256 manifest。
- `scripts/check-version-sync.sh` 要求 `package.json`、`package-lock.json` 顶层/根 package、Tauri 和所有 Cargo workspace package 只有一个版本；非 `unsigned` 发布标签还必须与该版本一致。本地打包、CI 和手动 release workflow 均在产物生成或验证前执行该检查。

## AI Toolbox 参考边界

`v0.5.0` Codex provider/gateway 切片只审阅 [AI Toolbox](https://github.com/coulsontl/ai-toolbox) 固定 commit `3f2faef5b7a453169d4325d89fab290154233723` 的行为、信息架构与失败模式；没有把上游作为构建依赖，也没有运行其 lifecycle script。上游根许可证是 MIT，但其 transformer 测试目录含从 AxonHub `llm` 同步的 LGPL-3.0 fixture，因此本项目明确不复制上游源码、transformer、fixture、样式、图标或品牌。

实现基于 OpenAI 官方 Responses、Codex config reference 与 App Server 契约 clean-room 编写。若未来直接复用上游任何代码或测试数据，必须先逐文件确认 provenance/许可证，更新 `THIRD_PARTY_NOTICES.md`、SBOM 与许可证清单，并重新运行依赖与供应链审计；AI Toolbox 根 MIT 不构成其嵌套来源的自动许可结论。

`v0.6.0` 首页 API 接入卡只把本机已安装 EasyCLIProxyAPI 的公开可见界面用作信息层级参考；该已发布版本没有复制或运行其源码、资源、样式、图标、品牌、协议转换或生命周期实现。后续未发布切片研究了 EasyCLIProxyAPI 固定 commit `79b50b7a2b76607e6ccd01966f4b6d4430a31dcd` 的 sidecar 产品行为，但 clean-room 实现自己的下载、校验、目录、进程和 UI 逻辑；EasyCLIProxyAPI 仍不是构建/运行依赖或派生代码来源。

## CLIProxyAPI 运行时依赖

未发布反代切片把 [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) 作为用户显式下载的独立预编译运行时，协议与桌面主体分别版本化；本仓库不编译、链接、复制或修改其 Go 源码，也不把二进制打入 Codex Manager 安装包。审计基线为 `v7.2.145`、tag commit `d9cea8904b14fbbebb77ef26e98ef08f6b48a724`，上游使用 MIT License。

内测安装器不请求 GitHub `releases/latest`，也不在运行时解析上游 `checksums.txt`。它只按 OS/arch 选择代码中已审核锁定的 `v7.2.145` 精确资产名与 SHA-256，从官方 tag URL 下载并校验。下载、解压和文件类型有界，安装使用应用私有 staging，失败保留旧 core。确认和安装必须由用户显式触发，不能由后台静默执行。

这不是完整供应链身份保证：上游没有 detached signature、Developer ID、公证、SBOM、Sigstore/SLSA provenance 或可复现构建证明，checksum 与 asset digest 也位于同一 GitHub 信任域；release workflow 还会从 mutable model catalog 拉取数据。主应用的 Developer ID/Tauri updater 签名不覆盖运行时下载的第三方二进制。内测固定 `v7.2.145`、tag commit、资产名称和 SHA-256；独立审核 manifest 与上游 provenance 属于后续供应链增强，不阻断内测迭代。

安全审计还确认 CLIProxyAPI OAuth callback 可绑定全网卡、OAuth token 会写入 auth-dir，Management API 可读取原始配置/认证文件并发起通用请求，且预编译版存在无法由当前配置关闭的后台版本请求。本集成只允许由原生层导入并隔离保存 CLIProxyAPI OAuth 档案，运行时投影到随机私有 auth-dir；Management API 与 OAuth callback 不向 WebView 开放，配置仍强制 loopback、`commercial-mode` 和日志/管理/插件关闭。

本地代理只承载 CLIProxyAPI OAuth 凭据池，不承担任意外部 Base URL 的通用转发；外部 API Key 供应商由 Codex 配置直接管理。首次正式发布前需在隔离环境完成一次网络与磁盘边界观测，后续仅在内核版本或配置边界变化时重复。

正式签名发布的 `verify-release.sh` 可检查 `.app`/`.dmg`、SBOM、许可证、notices 和资产摘要；内测 prerelease 只需应用测试、secret scan、固定 CLIProxyAPI 版本摘要和产物完整性，不要求上游 SBOM 或 provenance。

每个 beta 必须在 `BUILD-INPUT.json` 和 `BUILD-PROVENANCE.json` 中记录源码、workflow/run/attempt、build/sign runner image、macOS/Xcode、Node/npm/Rust/Cargo/Tauri/GitHub CLI/notarytool、Release ID、每个 core asset 的 ID/name/size/API SHA-256 digest/正式 tag URL、notary ID/status、app/DMG CDHash、Team/authority/leaf certificate hash 和 updater key hash。CI artifact 只证明构建完成，不等同于签名、notarized、草稿远端复验或公开发布。

Rust SBOM/license 依赖 `cargo-cyclonedx` 与 `cargo-license`。`cargo-cyclonedx 0.5.9` 不支持通用 `--output` 参数，脚本使用每个 manifest 的临时 override filename，将结果移入 `artifacts/release/sbom/`，并在成功或失败时清理精确临时文件。缺少工具时会留下明确的 `*-unavailable.txt`，不得误报为完整清单。

SBOM 内层 manifest 的路径相对仓库根目录，因此必须从仓库根目录运行 `shasum -a 256 -c artifacts/release/sbom/SHA256SUMS`。签名 Release 的扁平外层 manifest 覆盖九项资产中除 manifest 自身以外的八项文件，其中 `.app` 由 `.app.tar.gz` 的哈希覆盖；不能把 tarball 哈希表述为独立裸 `.app` 文件哈希。发布物固定位于 `artifacts/release/`，不得放在 Vite 会清空的 `dist/` 子树。

CI 通过 `.nvmrc` 固定 Node 24.18.0、使用 Rust 1.98.0，并固定发布工具版本：`cargo-audit 0.22.2`、
`cargo-cyclonedx 0.5.9`、`cargo-license 0.7.0`。`config/cargo-audit-allowlist.json` 逐项锁定 17 条 unmaintained 和 1 条 `glib` unsound warning 的 advisory/package/version/rationale；Linux GTK/glib 条目还必须持续不存在于 `aarch64-apple-darwin` 依赖图。任何新增 warning、yanked、vulnerability、版本变化或 target exclusion 失效都失败。升级工具或依赖时要重新审查 allowlist、输出格式和许可证。

Unsigned community prerelease 只能从同一 `main` commit 的成功 CI artifact 创建，并标记为 GitHub prerelease。允许上传 unsigned DMG、单独的 SBOM/许可证 ZIP 和针对最终上传资产生成的扁平路径 SHA-256 清单；禁止上传 `.app` 压缩包、完整 CI artifact、`latest.json`、`.app.tar.gz` 或 updater `.sig`。发布后必须回下载并逐字节验证资产，确保它不会进入稳定 updater 通道。该路径不接触 updater 私钥、Apple 凭据或签名 release workflow。

Tauri updater 的公钥可提交，私钥/密码不得进入仓库、缓存、artifact、SBOM 或日志。发布工作流分为三个权限域：secret-free `build` 生成并哈希 unsigned app/SBOM/license input；受保护 Environment 的 `sign` 只消费该 input、没有 `contents: write`，也不执行 input 携带的 signer 或 verifier。它使用 pinned Node setup，从 npm registry 安全重试下载固定的 `@tauri-apps/cli@2.11.4` 与 arm64 native package并逐项核对 SHA-512；updater 私钥只在紧邻签名命令的独立 step 注入，随后立即 unset。验签由 exact checkout 中只使用 Node `crypto` 的实现完成，transfer 只携带公开 key，不携带可执行验证器。之后在临时 Keychain 中完成 app/DMG 的 Developer ID、独立 `Accepted` 公证和 stapling，清理证书、Keychain 与 signer 后才上传 sealed artifact。

无 Apple/updater secret 的 `draft` 才取得 Release 写权限。GitHub eventual consistency 由统一 5 分钟 deadline、bounded exponential backoff 和严格 identity/digest reconciliation 处理；transient 404/429/5xx/网络错误可重试，未知/重复资产或 source/name/size/digest 冲突立即失败。资产记录按名称规范化后逐记录比较，数组枚举顺序本身不属于身份，但任何 name/size/digest/stable URL 变化都必须失败。`latest.json` 永远使用确定性的正式 tag URL `https://github.com/<owner>/<repo>/releases/download/v<VERSION>/<asset>`，draft 的 `untagged-*` URL 既不生成也不接受。草稿资产按 ID 原子回下载；公开后再匿名走全部正式 tag URL和 `releases/latest/download/latest.json` alias。GitHub attestation 必须锁定 source digest/ref/workflow/signer digest 并拒绝 self-hosted runner；attestation bundle 和 `RELEASE-VERIFICATION.json` 只保存到独立 Actions evidence artifact，避免 Release manifest 递归。

同一次 sign 的 sealed artifact 可通过 exact `resume_run_id` 与 `resume_release_id` 恢复，但 `RELEASE-INTENT.json`、source、Release identity 和所有已有 asset digest 必须完全一致；不自动删除、覆盖或重新签名。只读 `verify-release.yml` 不含 Apple secret、`contents: write` 或发布动作，可分别复验 existing draft 与人工发布后的公开 Release。

2026-08-29 当前依赖图的 `npm audit` 与 `cargo audit` 都报告 0 vulnerabilities；18 条 Cargo warning 已进入上述显式 allowlist。allowlist 是有理由、有目标图断言的发布门禁，不代表这些维护性风险已消失；每次公开发布仍需重新执行并审查变化。

## 未发布验收工具

调试 `desktop-qa` feature 直接复用工作区已有 `rusqlite` 版本来构造人工迁移前数据库，没有新增 crate 版本或第三方运行时；默认构建不启用该依赖入口，release + QA 明确拒绝编译。`npm run docs:check` 默认离线，扩充双语版本/平台/下载入口和本地链接/资产检查，联网 URL 检查须显式使用 `--check-external`。

已审核 CLIProxyAPI 的显式 ignored smoke 测试复用生产下载、校验、配置、健康和停止代码，安装到测试临时目录；不编译 Go、不改变审核基线，也不把内核打进应用包。真实空认证池观测与真实 OAuth 业务验收分别记录。
