# Codex Manager 发布运行手册

## 状态边界

1. `unsigned local build`：本机或 CI 生成 `.app`/`.dmg`，没有 Apple 签名。
2. `unsigned community prerelease`：从可追溯的 `main` commit 和成功 CI artifact 发布源码、unsigned DMG、SBOM、许可证与校验和；GitHub 必须标记 prerelease，并明确展示 Gatekeeper 警告。不得包含 `latest.json`、`.app.tar.gz` 或 updater `.sig`，不属于稳定在线更新。
3. `signed`：Developer ID Application 签名并启用 Hardened Runtime。
4. `notarized`：`xcrun notarytool` 获得 Apple accepted 结果。
5. `stapled`：`xcrun stapler staple` 固化 ticket，并执行 `stapler validate`。
6. `draft uploaded`：签名产物、Tauri updater 包/签名、`latest.json`、SBOM 和 provenance 已进入 GitHub 草稿 Release，仍不会被在线更新发现。
7. `published stable`：人工复核草稿和下载验收后发布；只有这个签名通道才允许提供 `latest.json` 并对客户端启用稳定在线更新。

`ci.yml` 默认只做 unsigned artifact，可用于人工创建上述 community prerelease。`release.yml` 只服务签名稳定通道：只能手动运行，并且只允许 `main` 当前远端 SHA；它使用受保护的 GitHub `release` Environment，凭据不齐时必须失败。没有证书、Team ID 和 notarization 凭据时，禁止模拟签名成功、绕过门禁或向 community prerelease 添加 updater manifest。

## 2026-08-29 v0.2.0 发布候选状态

- 源码版本已同步到 `0.2.0`。这个 minor 版本同时包含用户主动触发的 OAuth 账户/认证档案管理，以及活动 rollout 扫描被归档积压阻塞、界面不随后台采集重载的修复。
- 本机仍没有 Developer ID Application identity。GitHub `release` Environment 当前只有 Tauri updater 私钥及密码，缺少工作流要求的 7 项 Apple 证书、identity、Team ID 和 notarization 凭据；因此 signed stable 通道必须保持 No-Go，不触发注定失败的签名 workflow。
- 本版本只能从最终 `main` exact SHA 对应的成功 CI artifact 创建 `Unsigned Community Build` prerelease。发布前后必须执行本手册的 DMG、SBOM、许可证、SHA、source provenance 和回下载验收，并确认 Release 不含 updater 资产。
- `v0.1.0` 是独立的历史 community tag，不能覆盖。以后补齐 Developer ID 后必须使用高于 `0.2.0` 的新 SemVer 进入签名通道。

## Unsigned Community Build 发布

1. 只使用 `main` 当前 commit 的成功 `CI` run 所上传的 `codex-manager-macos-unsigned` artifact，不使用本机历史产物。
2. 验证 artifact 中的 `SHA256SUMS-unsigned`、SBOM 内层 manifest、DMG 的 `hdiutil verify`、版本和 `BUILD-SOURCE-unsigned.txt` source SHA。
3. GitHub Release 必须标记为 prerelease，标题和正文包含 `Unsigned Community Build`，并明确说明没有 Developer ID 签名、没有 Apple 公证、Gatekeeper 警告是预期行为。
4. 只上传 unsigned DMG、单独打包的 SBOM ZIP、单独打包的许可证 ZIP，以及针对这些最终上传资产重新生成的扁平路径 SHA-256 清单；源码归档由 GitHub 根据 tag 自动提供。CI 内层 `SHA256SUMS-unsigned` 含仓库相对路径，只作为构建证据，不能原样充当 Release 下载校验文件。
5. 发布后回下载每个资产并逐字节核对；确认 Release 中不存在 `latest.json`、`.app.tar.gz` 或 `.sig`，固定 updater endpoint 仍返回非 2xx。

该通道不是“正式 macOS beta artifact”，不能写成 signed、notarized、stapled、stable update 或普通用户无警告安装。后续签名版本必须提高 SemVer，不得用签名资产覆盖同一个 community tag。

## 2026-08-27 v0.1.0 历史状态

- 已完成 `unsigned local build`：`artifacts/release/Codex Manager.app` 和 `artifacts/release/Codex Manager_0.1.0_aarch64.dmg`。该目录与 Vite 的 `dist/` 分离，普通前端构建不得删除发布物。
- DMG 为 arm64、9.4 MiB，`hdiutil verify` 通过，SHA-256 为 `a66968a63b3d246f8a1a3ee6e61800e01c2adf94f8dd0e7bdd4be145117fff74`；以 `artifacts/release/SHA256SUMS-unsigned` 为准。
- SBOM、许可证清单和 `SHA256SUMS-unsigned` 完整性验证通过。
- 本机没有 Developer ID Application identity；strict `codesign` 和 Gatekeeper 拒绝 unsigned 产物是预期结果。未运行 notarization/stapling；这不阻止按上面的严格边界发布 community prerelease。
- 现有本机 unsigned 产物生成时仓库尚无 commit，`BUILD-SOURCE-unsigned.txt` 记录 `unversioned-working-tree`。首次提交之后，正式 beta 必须从可追溯 commit 重新构建。
- 应用代码已配置 GitHub `latest.json` endpoint 和 Tauri updater 公钥；生产构建只通过 `src-tauri/tauri.updater.conf.json` 开启 updater artifact，日常 unsigned 构建不会要求私钥。
- GitHub `release` Environment 已配置为只允许 `main`，并要求仓库所有者人工批准。`TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 已作为 Environment secrets 写入；私钥还有一份密码保护的仓库外本机副本，密码保存在 macOS Keychain 服务 `cc.codex.manager.updater`。
- Apple 的 `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`KEYCHAIN_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD` 和 `APPLE_TEAM_ID` 尚未配置，本机也没有 Developer ID identity。因此工作流当前会 fail closed；未触发 workflow，未创建草稿/公开 Release，线上 `latest.json` 尚不存在。
- updater 私钥的独立离线恢复备份尚未完成；首次公开发布前必须由发布人员保存到与当前主机/GitHub 分离的加密介质并验证可恢复性。

## 本地检查与构建

```sh
./scripts/check-version-sync.sh
./scripts/preflight-macos.sh
./scripts/package-macos.sh
./scripts/sbom.sh
./scripts/licenses.sh
./scripts/verify-release.sh unsigned
```

从仓库根目录验证 manifest：

```sh
shasum -a 256 -c artifacts/release/sbom/SHA256SUMS
shasum -a 256 -c artifacts/release/SHA256SUMS-unsigned
hdiutil verify "artifacts/release/Codex Manager_0.1.0_aarch64.dmg"
```

## 签名与公证（人工授权后）

```sh
codesign --deep --force --options runtime --timestamp --sign "Developer ID Application: <TEAM NAME> (<TEAM ID>)" path/to/Codex\ Manager.app
codesign --verify --deep --strict --verbose=2 path/to/Codex\ Manager.app
xcrun notarytool submit path/to/CodexManager.dmg --keychain-profile "<PROFILE>" --wait
xcrun stapler staple path/to/CodexManager.dmg
xcrun stapler validate path/to/CodexManager.dmg
```

凭证只能由发布人员通过 Keychain profile 或 CI secret 注入，不能写入仓库或日志。

完成签名后还必须重新生成对应版本标签的 manifest，不得沿用 unsigned 产物的 SHA-256。只有 `codesign --verify --deep --strict`、`notarytool` accepted、`stapler validate` 和 Gatekeeper 全部通过后，才能将状态提升为 `stapled`。

## GitHub Releases 更新发布

1. 在 `main` 上同步 `package.json`、`package-lock.json`、workspace/Cargo 和 `tauri.conf.json` 的 SemVer，并保证当前远端 SHA 是唯一发布源。
2. 在 GitHub Actions 手动运行 `Release macOS arm64 updater`，输入完全相同的版本。任务会先运行版本、npm/Rust 门禁、secret scan、audit、SBOM 和许可证清单。
3. 指定发布人在 `release` Environment 审批页面核对 SHA 和版本后才批准凭据解锁。工作流先用私钥签名探针确认它与仓库配置的 updater 公钥配对，再把 `.p12` 导入 runner 的临时 Keychain 并核对精确 identity；临时 release config 会把该 identity 显式写入 Tauri 的 `bundle.macOS.signingIdentity`，随后用完整 SHA 固定的 `tauri-action` 构建 macOS arm64。Tauri 在同一次 bundle 中完成 `.app` 的 Developer ID 签名、notarization/stapling，随后才从该 `.app` 生成并签名 updater tarball，只创建 draft Release。工作流再对 DMG 独立执行 `notarytool submit --wait`、确认 accepted、staple，并以处理后的同名 DMG 覆盖草稿资产、回下载逐字节核对；这不会改写 updater tarball 的来源 `.app`。
4. 工作流必须确认 `.app.tar.gz`、`.sig`、DMG 和 `latest.json` 存在；`latest.json.version` 与源码一致，平台键精确为 `darwin-aarch64` 和 `darwin-aarch64-app`，两者的 URL 都必须等于该版本草稿 Release 中 tarball asset ID 对应的 GitHub API URL，且 signature 与 `.sig` 内容逐字一致。Tauri 下载时会为这类 URL 发送 `Accept: application/octet-stream`。
5. 发布人在草稿页面复核 source SHA、codesign identity、notary/staple/Gatekeeper、哈希、SBOM、许可证和 provenance，下载草稿产物到隔离机验收。未获得发布授权时停在 draft。
6. 人工发布后，使用上一个已签名版本实际点击“检查更新”，验证它只发现更高 SemVer，显示纯文本说明，并在原生确认、签名校验、安装和重启后报告 `accepted`。

私钥不做例行轮换。正常轮换必须先用旧 key 签名一个内嵌新公钥的桥接版本，确认旧客户端安装后再切换。如旧 key 已泄露，立即停止在线更新，不得再信任旧 key 签名的桥接包；改为人工分发嵌入新公钥的 Developer ID 签名且已公证版本。

## 回滚

保留 artifact、manifest、SBOM 和 commit provenance。发现问题时撤下 beta 下载入口并公告受影响版本，不要覆盖原 tag/asset/SHA。在线更新只接受更高 SemVer；回滚代码也必须发布为新的更高版本，例如将 `0.1.2` 的回退代码发布为 `0.1.3`。
