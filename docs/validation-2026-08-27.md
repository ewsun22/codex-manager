# P1–P4 本机验证记录

日期：2026-08-27
状态：implemented、verified locally、unsigned packaged、GitHub updater infrastructure configured；未 signed、未 notarized、未 draft/released

## 环境

- macOS 27.0，Apple Silicon `arm64`。
- Node.js 24.18.0，npm 11.16.0。
- Rust/Cargo/rustc 1.98.0。
- Codex Desktop bundled CLI 0.150.0-alpha.8；界面 capability probe 只保存版本和 Schema SHA-256。
- `security find-identity -p codesigning` 未发现可用 Developer ID Application identity。

## 代码与数据层门禁

以下命令在当前工作树实际运行并通过：

```bash
npm run check
./scripts/check-version-sync.sh
cargo check --workspace --all-targets
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./scripts/audit-dependencies.sh
./scripts/scan-secrets.sh
```

实际结果：

- Node/TypeScript：严格类型检查、30/30 测试通过，脱敏 rollout probe 通过。
- Rust：workspace check 通过，49/49 测试通过（core 7、desktop 37、storage 5），format 和 `-D warnings` clippy 通过。
- 安全回归覆盖 OTel 未认证请求的解码前拒绝、空闲连接配额释放、不受控元数据不落库、token 一致性/溢出、同内容异 inode 副本查询折叠、source ownership 隔离、路径竞态、AGENTS atomic-swap CAS，以及更新说明截断和预期版本校验。
- `npm audit` 为 0 vulnerabilities；`cargo audit` 为 0 vulnerabilities，输出 18 条已知 warning，主要来自 Linux GTK 间接链的 unmaintained advisory，以及 `proc-macro-error`、`rustls-pemfile`、`unic` 和 `glib` 的已知 warning；未将 warning 冒充为漏洞清零。
- secret scan 通过，未发现要提交的高风险凭据格式。

## 安全扫描与修复验证

对完整代码树运行了标准安全扫描，共确认 12 项发现（9 medium、3 low）。修复后由新的只读验证者逐项追踪，结论为 `verified_fixed`，无新 blocker。扫描使用量为 28,428,699 tokens：input 28,245,523（其中 cached 26,958,336），output 183,176，reasoning 78,452。

修复覆盖：OTel TLS/header 认证与资源配额、项目发现/采集预算、workflow shell 输入边界、source namespace 所有权、token 一致性和上限、endpoint 密钥化、随机持久端口 fail-closed、逐段 no-follow 文件读取、AGENTS CAS 竞态与 Node probe 单行内存上限。

## 浏览器业务链

Playwright 对 `http://127.0.0.1:1420/` 的人工脱敏 demo adapter 做了可视化验收：

- 1440×1000/1080：总览、活动搜索+失败筛选、调用详情、项目切换、顶级全局 AGENTS 默认只读展示、项目根 AGENTS 绿/黄状态点、创建后即时状态同步、AGENTS 脏状态/安全保存/revision、设置保存和 OTel 配置显式揭示均通过。
- 390×844：主导航、活动筛选和 OTel 设置可用；`documentElement.scrollWidth = clientWidth = 390`，没有全局横向溢出。
- 应用更新卡片的未检查 → 发现 `0.2.0` → 演示安装状态链通过；桌面布局和 390×844 窄屏按钮/发布说明可见，窄屏实测 `scrollWidth = clientWidth = 375`。
- 控制台 0 error、0 warning；未发现失败的动态请求。
- 证据位于 `output/playwright/final-*.png`、`output/playwright/agents-global-project-status.png`、`output/playwright/agents-create-status-sync.png`、`output/playwright/online-update-desktop.png` 与 `output/playwright/online-update-mobile.png`。演示模式的 AGENTS 保存只写入浏览器内存，更新也不下载真实产物。

## GitHub 在线更新基础设施

- `tauri-plugin-updater 2.10.1` 和 Rust-only command 封装已编译通过；capability 只新增 `allow-check-for-update` 和 `allow-install-pending-update`，未授予 `updater:default`、dialog、HTTP 或 process 插件权限。CSP 未放宽。
- 日常构建配置包含固定 GitHub endpoint 和 updater 公钥；只有 `tauri.updater.conf.json` 打开 `createUpdaterArtifacts`，解析/合并配置检查通过。
- `npm run tauri build -- --debug --no-bundle --config src-tauri/tauri.updater.conf.json` 实际执行成功，证明 release-only config 路径、Tauri 插件初始化和合并后应用构建可用；`--no-bundle` 没有生成/签名 updater artifact，因此不是发布证据。
- 使用 macOS Keychain 中的密码对仓库外 updater 私钥完成一次临时文件签名探针；签名生成成功，配置公钥与 `.pub` 文件逐字一致。临时探针已移到用户废纸篓，可恢复/清空。
- GitHub API 实查 `release` Environment 已有 required reviewer 和 `main` custom branch policy，两个 `TAURI_SIGNING_*` Environment secrets 已存在。仓库 Release 数量为 0，未触发 workflow。
- `.github/workflows/release.yml` 已通过独立工作流子任务的 YAML/actionlint 静态检查和所有第三方 action SHA 远端核对。最终复审发现证书导入、signing identity 注入、updater archive 时序、GitHub asset URL 格式和 DMG stapling 问题；工作流已改为临时 Keychain 导入/精确 identity 检查、临时 Tauri release config 显式注入 identity，并由同一 Tauri bundle 先完成 app 签名/公证/staple、后生成 updater tarball。DMG 另行执行 notarytool/staple 并覆盖同名草稿资产，manifest URL 则精确绑定草稿 tarball 的 GitHub API asset ID。两个 checkout 禁止持久化 GitHub token，Environment 私钥还会先签名探针并由配置公钥验签。修正后重新通过 YAML、diff 和 config 校验。这是静态证据，不等于真实签名/公证/草稿上传。

## macOS unsigned 产物

以下命令实际运行并通过：

```bash
./scripts/preflight-macos.sh
./scripts/package-macos.sh
./scripts/sbom.sh
./scripts/licenses.sh
./scripts/verify-release.sh unsigned
```

产物与验证：

- `artifacts/release/Codex Manager.app`：`arm64` Mach-O，bundle id `cc.codex.manager`，版本 `0.1.0`，最低 macOS `13.0`。
- `artifacts/release/Codex Manager_0.1.0_aarch64.dmg`：9.4 MiB，`hdiutil verify` 通过。
- DMG SHA-256：`a66968a63b3d246f8a1a3ee6e61800e01c2adf94f8dd0e7bdd4be145117fff74`；以 `artifacts/release/SHA256SUMS-unsigned` 为准。打包后再次运行 `npm run build`，DMG 与外层 manifest 哈希均未变，证明 Vite 不再删除发布目录。
- npm、Cargo workspace 与 Tauri 版本统一为 `0.1.0`；npm 一份与 Rust 三个 workspace package 的 CycloneDX JSON、Cargo/npm 许可证清单均可解析；`artifacts/release/sbom/SHA256SUMS` 和 `artifacts/release/SHA256SUMS-unsigned` 都从仓库根目录验证通过。`verify-release.sh` 现在会对缺失或 unavailable 的 SBOM/许可证清单直接失败，CI 在上传前也执行同一门禁。
- `BUILD-SOURCE-unsigned.txt` 如实记录 `source=unversioned-working-tree`、`signing=unsigned`。生成该产物时仓库尚无 commit，所以这不是可追溯的正式发布物。

`codesign` 只能看到 Rust 链接器留下的 ad-hoc executable signature，`TeamIdentifier` 缺失；对 `.app` 的严格签名验证和 Gatekeeper 验证失败是未签名状态下的预期结果。未运行 notary submission 或 stapling，未上传或发布任何产物。

## 历史 Phase 0 真实 rollout 探针

开发初期曾对一份仍在增长的 Codex Desktop rollout 执行一次只读、metadata-only 快照探针：4,714 条完整 JSONL 行、5 个 turn、694 个累计向量增长记录、2,362 个 message/tool/reasoning 元数据项、23 个累计向量不变的重复快照，解析诊断为 0。该探针没有输出 ID、完整路径或消息正文。

这份历史证据只证明当时本机内部 JSONL Schema 的可观测性，不把累计向量增长宣称为网络请求数，也不把内部 JSONL 视为稳定公开 API。

## 剩余边界

- 真实 wire response bytes、代理层 TTFB、完整请求 URL 和供应商账单仍不可从当前原生来源可靠得到，UI 必须显示 `unavailable`。
- rollout JSONL 仍是内部兼容来源，版本升级后需重新验证。
- Windows 还未实现 reparse-point 等价安全、安装包与签名验收。
- Developer ID 签名、Apple notarization/stapling、公开 beta 发布和下载验证尚未执行。
- Apple 发布凭据和 updater 私钥的独立离线恢复备份尚未配置；首次公开 Release 前都是硬门禁。
- 实际端到端更新无法在 Release 数量为 0 时验收；必须从一个旧的已签名/已公证版本升级到已人工发布的更高 SemVer 才能标记 accepted。
