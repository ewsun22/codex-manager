# Codex Manager 发布运行手册

## 状态边界

1. `unsigned local build`：本机或 CI 生成 `.app`/`.dmg`，没有 Apple 签名。
2. `unsigned community prerelease`：从可追溯的 `main` commit 和成功 CI artifact 发布源码、unsigned DMG、SBOM、许可证与校验和；GitHub 必须标记 prerelease，并明确展示 Gatekeeper 警告。不得包含 `latest.json`、`.app.tar.gz` 或 updater `.sig`，不属于稳定在线更新。
3. `signed`：Developer ID Application 签名并启用 Hardened Runtime。
4. `notarized`：`xcrun notarytool` 获得 Apple accepted 结果。
5. `stapled`：`xcrun stapler staple` 固化 ticket，并执行 `stapler validate`。
6. `draft uploaded`：九项签名资产已进入唯一 GitHub 草稿 Release；这只描述上传状态。
7. `draft remotely verified`：九项资产已按 asset ID 回下载，并通过哈希、provenance、attestation、updater 签名、app/DMG 签名、公证、stapling 与 Gatekeeper 全链复验；草稿仍未公开。
8. `published publicly verified`：人工 Publish 后，Release、`releases/latest`、tag 与 exact source SHA 一致，九个正式 tag URL 和 `latest.json` alias 均匿名回下载并重跑完整门禁。
9. `updater E2E accepted`：已有可信签名客户端真实下载更高 SemVer、验签、安装并重启成功。首个签名版本无法满足这项状态。

`ci.yml` 默认只做 unsigned artifact，可用于人工创建上述 community prerelease。`release.yml` 只服务签名稳定通道：只能手动运行，并且只允许 `main` 当前远端 SHA；它使用受保护的 GitHub `release` Environment，凭据不齐时必须失败。没有证书、Team ID 和 notarization 凭据时，禁止模拟签名成功、绕过门禁或向 community prerelease 添加 updater manifest。

## 2026-08-29 v0.2.1 签名候选状态

- `v0.2.0` 已从 `87099222b0e9a50c4410a040c9600e23c53afb23` 发布为公开 `Unsigned Community Build` prerelease，只含 unsigned DMG、SBOM、许可证和 community SHA manifest；2026-08-29 回下载四项资产后，外层 SHA manifest、DMG `hdiutil verify` 均通过，稳定 `latest.json` endpoint 返回 404。该 tag/Release 不得覆盖或补入 updater 资产。
- 签名候选源码已提高到 `0.2.1`，避免复用公开的 `v0.2.0` tag。它包含同一应用功能以及本次签名、notarization、stapling、provenance 和远端复验工作流加固。
- 本机仍没有 Developer ID Application identity，但 GitHub `release` Environment 已配置工作流要求的 6 项 Apple secret 和 2 项 Tauri updater secret，并保持 `main` custom branch policy 与人工 reviewer 门禁。发布人员已报告 updater 私钥的独立离线备份完成；仓库和工作流只记录 secret 名称，不记录值。
- 七次失败 run 的根因依次是 runner 缺少 `rg`、Hardened Runtime 输出解析错误、DMG detach 目标错误、用 tag REST 读取 draft、创建后的 Release 列表传播延迟、读取 stale 的 Release 内嵌 assets、以及固定 20 秒资产可见性窗口。它们说明 GitHub Release/asset/attestation 必须按 eventual-consistency 状态机处理，不能继续追加固定次数轮询。
- Actions run `33227321418` / source `91d5081b1ffbb3b837dfc1776ec8ca8d86d25de9` 生成的签名产物本体已由仓库外总审计验证：app/DMG 的 Developer ID、Hardened Runtime、secure timestamp、Apple `Accepted`、stapling、Gatekeeper、updater 签名、SBOM、哈希与九项 attestation 都成立。该结论只绑定这个 exact SHA 和字节。
- 同一 run 的 secret gate 实际因缺少 `rg` 被 `|| true` 静默吞掉；现有 draft `378845893` 的 `latest.json` 又包含永久失效的 `untagged-*` URL。该 draft 保持 `draft=true`、无 `v0.2.1` tag，明确为 **No-Go**，不得发布、修补后冒充成功或写成 `remote verified/accepted`。
- 当前工作树中的状态机、scanner、provenance 和 post-publish 修复仍是本地未提交准备；尚未 commit、push、dispatch 或改变 draft。只有最终新 SHA 完成本手册全部门禁后，才可更新状态。

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
- 以上是 2026-08-27 的历史准备状态；到 2026-08-29，GitHub `release` Environment 已补齐 6 项 Apple secret，真实 sign job 已通过。临时 Keychain 密码仍由 runner 随机生成，本机仍没有 Developer ID identity，二者都不影响受保护 runner 的已验证签名链。
- 以上是 2026-08-27 的历史备份状态；发布人员随后报告 updater 私钥的独立离线备份已完成。首次人工发布前仍需按实际恢复介质执行发布人复核，不把口头确认替代远端资产验签。

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
2. 在 GitHub Actions 手动运行 `Release signed macOS arm64`，输入完全相同的版本。`preflight` 先拒绝非当前远端 `main`、已存在 tag/Release 或版本不一致，再由完全不接触 secret 的 `build` job 运行 npm/Rust、secret scan、audit、SBOM/许可证和 unsigned `.app` 构建，产出带 SHA-256 的不可变 signing input。
3. 指定发布人在 `release` Environment 审批页核对 exact SHA 和版本后才批准 `sign` job。它在审批后再次读取远端 `main` 与 tag/Release，随后从 npm registry 下载固定版本 Tauri signer 并核对 `package-lock.json` 固化的两个 SHA-512；不执行 build artifact 中的 signer。之后才把 `.p12` 导入临时 Keychain 并立即删除证书文件；runner 随机生成临时 Keychain 密码。这个 job 不拥有 `contents: write`，不重新运行项目 build script；它只对已经哈希的 `.app` 做 Developer ID/Hardened Runtime/secure timestamp 签名，分别以 `notarytool --output-format json` 证明 app 和 DMG 为 `Accepted`，staple 后生成并验签 updater tarball。
4. Apple/app-specific password 先写入临时 Keychain profile，提交时只引用 profile；完成所有签名后先恢复原 Keychain 列表、锁定并删除临时 Keychain，确认 `.p12`/Keychain/列表快照都不存在，才允许上传只含公开产物的 sealed workflow artifact。
5. 无 Apple/updater secret 的 `draft` job 才取得 `contents: write`。所有 GitHub API 请求固定 `X-GitHub-Api-Version`；Release 创建、唯一性、资产可见性、按 ID 下载与 attestation 使用统一 5 分钟 deadline 和有上限的指数退避。只对 404、429、5xx、超时和网络失败重试；未知资产、重复资产、identity/source/size/digest 冲突立即失败。下载先写 partial 再原子替换；上传响应不确定时仅按 exact name/size/API SHA-256 调和，绝不 `--clobber`。
6. `latest.json` 在 draft 阶段就写入确定性的正式地址 `https://github.com/<owner>/<repo>/releases/download/v<VERSION>/<tar-name>`。draft 的 `untagged-*` URL 永远不得进入 manifest；draft 内容只能按 asset ID 验 bytes 和字符串契约。`BUILD-PROVENANCE.json` 同时嵌入 build/sign toolchain、runner image/Xcode/Node、workflow/run/attempt、Release 与 core asset ID/name/size/API digest/稳定 URL、notary ID/status、CDHash、Team/authority/leaf certificate hash 和 updater key hash。
7. 工作流按 asset ID 回下载九项资产，重新执行外层/内层 SHA、updater 密码学验签、严格 metadata schema、SBOM/许可证 JSON、app/DMG `codesign`、Hardened Runtime、Team/leaf certificate、secure timestamp、无禁用 entitlement、`stapler validate`、`spctl`、`hdiutil verify` 与 embedded app CDHash。`gh attestation verify` 还必须锁定 source/ref/workflow/signer digest 并拒绝 self-hosted runner；bundle 与 `RELEASE-VERIFICATION.json` 只进入独立 Actions evidence artifact，不回写 Release 造成递归 manifest。
8. transient failure 后可同时填写 exact `resume_run_id` 和 `resume_release_id`。只有 existing draft 的 source、release ID、sealed `RELEASE-INTENT.json`、build/sign evidence 和每个已上传资产 digest 全匹配时才续传/重验；任何冲突失败，不自动删除、覆盖或重新签名。当前旧 draft `378845893` 绑定旧 SHA 和错误 metadata，不能被新 SHA resume。
9. 发布人在草稿页面和隔离机复核 source SHA、notary submission、Gatekeeper、哈希、SBOM、许可证、provenance 与 attestation。未获得发布授权时停在 draft；失败的 draft/tag 不自动处置。
10. 人工 Publish 后运行只读 `Verify existing macOS Release` 的 `published` 模式。它必须确认 `draft=false`、`prerelease=false`、tag 与 `releases/latest` 都绑定 exact Release/source，再匿名下载九个正式 tag URL 和 `/releases/latest/download/latest.json`，重跑第 7 步并保存 evidence。只有这样才能写成 `published publicly verified`。
11. 对 `v0.2.1` 另做干净 macOS 环境的 DMG quarantine、拖入 `/Applications`、启动与重启验收。由于 `v0.2.0` 无可信签名，真实 updater E2E 必须留到从 `v0.2.1` 升级到更高 SemVer；此前不能报告 `updater E2E accepted`。

私钥不做例行轮换。正常轮换必须先用旧 key 签名一个内嵌新公钥的桥接版本，确认旧客户端安装后再切换。如旧 key 已泄露，立即停止在线更新，不得再信任旧 key 签名的桥接包；改为人工分发嵌入新公钥的 Developer ID 签名且已公证版本。

## 回滚

保留 artifact、manifest、SBOM 和 commit provenance。发现问题时撤下 beta 下载入口并公告受影响版本，不要覆盖原 tag/asset/SHA。在线更新只接受更高 SemVer；回滚代码也必须发布为新的更高版本，例如将 `0.1.2` 的回退代码发布为 `0.1.3`。
