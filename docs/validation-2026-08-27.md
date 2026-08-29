# P1–P4 本机验证记录

日期：2026-08-27
状态：implemented、verified locally、unsigned packaged、GitHub updater infrastructure configured；未 signed、未 notarized、未 draft/released

> 状态更新（2026-08-29）：上面一行及下方 `v0.2.1` 候选审计是历史快照。`v0.2.1` 后续已从 exact source `fbe42de828f4f64549d9a4b36bc506fd15b11aa2` 完成签名、公证、stapling、公开发布和匿名回下载复验；对应 release run 为 `33240800663`，published verify run 为 `33241992763`。当前 `v0.2.2` 发布边界以 [发布运行手册](release.md) 和 [v0.2.2 发布说明](release-notes/v0.2.2.md) 为准。

## 环境

- macOS 27.0，Apple Silicon `arm64`。
- Node.js 24.18.0，npm 11.16.0。
- Rust/Cargo/rustc 1.98.0。
- Codex Desktop bundled CLI 0.150.0-alpha.8；界面 capability probe 只保存版本和 Schema SHA-256。
- `security find-identity -p codesigning` 未发现可用 Developer ID Application identity。

## 2026-08-29 v0.2.1 发布总审计追加

- 总审计绑定远端 source `91d5081b1ffbb3b837dfc1776ec8ca8d86d25de9`、Actions run `33227321418` 和唯一 draft Release `378845893`。九项 asset 的 API digest、本地 SHA、sealed transfer、updater 公钥/签名、app/DMG Developer ID/Hardened Runtime/timestamp、notary `Accepted`、stapling、Gatekeeper、SBOM/许可证与 attestation 已通过仓库外逐项复验；这是旧 SHA 的产物本体证据。
- 该 run 的 secret step 在真实日志中出现 `rg: command not found` 后仍返回 success；draft 的 `latest.json` 又使用发布后仍为 404 的 `untagged-*` URL。因此 draft `378845893` 是明确 **No-Go**；它后来在 exact ID/source、九项 asset、无 tag 和恢复 artifact 复核后被精确删除，从未发布。
- release 控制面修复已作为 `606a927c4f536b0f15a22a8ec850b6c99a97a117` 推送到 `main`：稳定 tag URL、统一 5 分钟 deadline/指数退避、exact release/asset reconcile、原子下载、完整 provenance、恢复路径和只读 post-publish verifier；secret scan 改为无 `rg` 依赖并以 `0/1/2` 表示 clean/finding/scanner error。run `33239136597` 已实测通过这些无密钥门禁。
- 最终独立安全审查发现 build 产出的 updater verifier 曾跨入 sign/draft 权限域执行；本地候选已移除该可执行传递，改为 exact checkout 中仅依赖 Node `crypto` 的 Minisign/Ed25519 verifier，并用人工签名、篡改 fixture 及旧 draft 的真实签名字节同 Rust verifier 交叉验证。审查同时复现的 encrypted PKCS8、YAML block、Bearer `+`/`/`/`=` 与尾随空白证书后缀绕过已加入 CLI 回归测试。新 SHA 仍须重新跑远端门禁，不能从本地结果获得 signed/accepted 状态。
- run `33239136597` 的 sign job 已实测 app/DMG 签名、Hardened Runtime、Apple `Accepted`、stapling、Gatekeeper、updater 验签和本地全链通过，并在上传 sealed signed-transfer 前清理临时 Apple 凭据。draft job 随后在任何 Release 创建前因 core asset 数组排序规则不一致失败：六项记录逐名称、size、digest、stable URL 规范化后完全相同。当前本地回归修复使记录身份与数组顺序无关，同时继续拒绝任意 size/digest 篡改；未获得新的远端授权前不提交、不推送、不二次 dispatch。

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
- 当日 GitHub API 实查 `release` Environment 已有 required reviewer 和 `main` custom branch policy，两个 `TAURI_SIGNING_*` Environment secrets 已存在；当时仓库 Release 数量为 0。此条是 2026-08-27 历史快照，后续已发布 `v0.1.0` unsigned prerelease，不能用它判断当前线上状态。
- 八次失败链已完整归因：前七项为缺少 `rg`、codesign runtime 输出解析、DMG detach、draft tag REST 404、Release 创建后 list 延迟、stale 内嵌 assets，以及最终资产超过固定 20 秒窗口；第八项是 intent 生成与文件复验使用不同字符串排序规则，却把资产数组顺序误当成身份。旧的局部补丁、固定次数轮询和只测 Schema 不执行真实文件路径都不再作为完成证据。
- 新控制逻辑拒绝 `browser_download_url`/`untagged-*` manifest，使用正式 `v<VERSION>` URL；所有 Release/asset/attestation 传播使用统一 deadline，冲突立即失败。恢复只接受 exact prior run/release/intent/digest；已删除 draft `378845893` 与未曾创建 draft 的 run `33239136597` 都不能通过现有双 ID 入口复用。

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
- Developer ID/Hardened Runtime、Apple notarization `Accepted`、stapling、Gatekeeper 和 updater 验签最近在 `606a927c4f536b0f15a22a8ec850b6c99a97a117` 的 sealed signed-transfer 上成立；但 run 在创建 draft 前失败，当前没有 v0.2.1 Release/tag，因此仍为 No-Go。新的控制修复和 exact SHA 必须重新通过远端 draft 全链，不能把 signed-transfer 写成 `draft remotely verified` 或 `accepted`。
- GitHub `release` Environment 的 Apple/Tauri secret 名称与门禁已配置，发布人员报告 updater 私钥独立离线备份完成；secret 值和备份内容不进入仓库或验证记录。首次公开 Release 仍需发布人复核实际恢复介质和 draft 证据。
- `v0.2.0` 后续已作为 unsigned community prerelease 发布，不能充当可信 updater 起点；必须从一个旧的已签名/已公证版本升级到已人工发布的更高 SemVer，才能把端到端 updater 标记为 accepted。
- 2026-08-29 对公开 `v0.2.0` 重新下载四项资产：`SHA256SUMS-v0.2.0-community` 全部通过，DMG `hdiutil verify` 通过，资产列表中没有 `latest.json`、`.app.tar.gz` 或 updater `.sig`，稳定 `latest.json` endpoint 返回 404。
