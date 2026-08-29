# 供应链与可复现发布

- `package-lock.json` 与 `Cargo.lock` 必须进入版本控制；CI 使用 `npm ci`。
- GitHub Actions 第三方 action 以完整 commit SHA 固定，并在行尾注释 tag。
- 手动 release workflow 只接受 `main` 当前 SHA，使用有 required reviewer 和 main-only 规则的 `release` Environment；Tauri/Apple 私密凭据只存在 Environment secrets。缺少任一凭据必须 fail closed。
- 发布前运行 `npm audit`、`cargo audit`（环境安装时）。
- `scripts/scan-secrets.sh` 对仓库文件做高风险凭据格式回归检查，仅输出命中文件名。
- `scripts/sbom.sh` 生成 npm 应用 SBOM，并为 `codex-core`、`codex-storage`、`codex-manager-desktop` 三个 Rust package 分别生成 CycloneDX JSON；`scripts/licenses.sh` 生成许可证清单。
- `scripts/verify-release.sh` 为每个构建目录生成 SHA-256 manifest。
- `scripts/check-version-sync.sh` 要求 `package.json`、`package-lock.json` 顶层/根 package、Tauri 和所有 Cargo workspace package 只有一个版本；非 `unsigned` 发布标签还必须与该版本一致。本地打包、CI 和手动 release workflow 均在产物生成或验证前执行该检查。

`verify-release.sh` 不只检查 `.app`/`.dmg`；它还强制要求 npm 与三个 Rust package 的 SBOM、内层 SBOM 哈希、Cargo/npm 许可证 JSON 和 notices，并解析 JSON、验证内层 manifest。任一文件缺失或出现 `*-unavailable.txt` 都必须失败。`main` push 的 CI 在上传 unsigned artifact 前安装固定版本工具、生成这些清单，再执行验证。

每个 beta 应记录源码 commit、Node/Rust/toolchain 版本、构建平台、构建时间、SBOM
及 `SHA256SUMS-*`。CI artifact 只证明构建完成，不等同于签名、notarized 或上传发布。

Rust SBOM/license 依赖 `cargo-cyclonedx` 与 `cargo-license`。`cargo-cyclonedx 0.5.9` 不支持通用 `--output` 参数，脚本使用每个 manifest 的临时 override filename，将结果移入 `artifacts/release/sbom/`，并在成功或失败时清理精确临时文件。缺少工具时会留下明确的 `*-unavailable.txt`，不得误报为完整清单。

SBOM 内层 manifest 的路径相对仓库根目录，因此必须从仓库根目录运行 `shasum -a 256 -c artifacts/release/sbom/SHA256SUMS`。外层 `SHA256SUMS-*` 覆盖 app、dmg、SBOM、许可证和 provenance，也从仓库根目录验证。发布物固定位于 `artifacts/release/`，不得放在 Vite 会清空的 `dist/` 子树。

CI 固定使用 Rust 1.98.0，并固定发布工具版本：`cargo-audit 0.22.2`、
`cargo-cyclonedx 0.5.9`、`cargo-license 0.7.0`。升级时要重新验证输出格式和许可证。

Unsigned community prerelease 只能从同一 `main` commit 的成功 CI artifact 创建，并标记为 GitHub prerelease。允许上传 unsigned DMG、单独的 SBOM/许可证 ZIP 和针对最终上传资产生成的扁平路径 SHA-256 清单；禁止上传 `.app` 压缩包、完整 CI artifact、`latest.json`、`.app.tar.gz` 或 updater `.sig`。发布后必须回下载并逐字节验证资产，确保它不会进入稳定 updater 通道。该路径不接触 updater 私钥、Apple 凭据或签名 release workflow。

Tauri updater 的公钥可提交，私钥/密码不得进入仓库、缓存、artifact、SBOM 或日志。发布工作流分为三个权限域：secret-free `build` 生成并哈希 unsigned app/SBOM/license input；受保护 Environment 的 `sign` 只消费该 input、没有 `contents: write`，也不执行 input 携带的 signer。它先从 npm registry 下载固定的 `@tauri-apps/cli@2.11.4` 与 arm64 native package，逐项核对 lockfile 固化的 SHA-512；updater 私钥只在紧邻签名命令的独立 step 注入，随后立即 unset。之后在临时 Keychain 中完成 app/DMG 的 Developer ID、独立 `Accepted` 公证和 stapling，清理证书、Keychain 与 signer 后才上传 sealed artifact。无 Apple/updater secret 的 `draft` 才取得 Release 写权限。它通过分页 Release 列表检测包括 draft 在内的同名版本，使用 REST 创建调用直接返回的唯一 Release ID，并在创建后复查同 tag 唯一性；资产只通过该 ID 的 `upload_url` 上传并逐项复查 asset ID/name/state/size，避免 tag REST 对 draft 返回 404 和按 tag 解析歧义。草稿阶段按 asset ID 回下载全部资产，验证实际 updater 签名、DMG 与 tarball 内 app、SBOM/许可证、完整外层 manifest、provenance 和 GitHub artifact attestation。任何已存在 tag/Release 都拒绝覆盖。

2026-08-27 本机验证已将 npm、Cargo 和 Tauri 版本统一为 `0.1.0`，生成 4 份可解析 CycloneDX JSON、Cargo/npm 许可证 JSON 和 notices，两层 manifest 校验通过。`npm audit` 与 `cargo audit` 都报告 0 vulnerabilities；Cargo 仍有 18 条可见 warning，应在每次公开发布前重新审查，不能被静默忽略。
