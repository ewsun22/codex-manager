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

Release note 采用 `docs/release-notes/v<VERSION>.md` 的固定章节。CI 通过 `npm run docs:check` 校验章节和状态词；Release 正文直接使用完整 note，`scripts/release-notes.mjs` 从其中的“概览”和“主要变化”生成 `latest.json.notes` 短摘要。两者必须来自同一文件，摘要不包含凭据或隐私数据。

`ci.yml` 的 push 触发只接受 `main`，避免创建版本 tag 时对同一 SHA 重复运行高成本验证；它默认只做 unsigned artifact，可用于人工创建上述 community prerelease。`release.yml` 只服务签名稳定通道：只能手动运行，并且只允许 `main` 当前远端 SHA；它使用受保护的 GitHub `release` Environment，凭据不齐时必须失败。没有证书、Team ID 和 notarization 凭据时，禁止模拟签名成功、绕过门禁或向 community prerelease 添加 updater manifest。

## 2026-08-29 v0.3.0 已发布事实

- `v0.2.0` 是从 `87099222b0e9a50c4410a040c9600e23c53afb23` 发布的公开 `Unsigned Community Build` prerelease，不含 updater 资产，仍与签名通道隔离。
- `v0.2.1` 已从 exact source `fbe42de828f4f64549d9a4b36bc506fd15b11aa2` 完成签名发布：run `33240800663` 通过 Developer ID、双 notary `Accepted`、stapling、Gatekeeper、updater 签名、SBOM、provenance、attestation 与草稿全量回下载门禁；公开后只读 run `33241992763` 又对正式 tag URL、`releases/latest` 和 `latest.json` alias 完成匿名复验。
- `v0.2.2` 已从 exact source `a2359e20eda9423f509a6a76f59d3e4c61a10ddc` 完成签名稳定发布；release run `33245641061` 与 published verify run `33246307716` 均成功，Release ID 为 `378939150`，九项资产已公开且 tag/source 一致。现有 `v0.2.2` tag 和资产不可覆盖。
- `v0.3.0` 包含活动记录粒度与终态、canonical 指纹 v2、schema v9、部分价格覆盖、项目最后对话时间、分页与开源文档流程，已从 exact source `fab415af2d9da6dfa41e79c7073bfb9fbb5fb818` 完成签名发布。
- `v0.3.0` release run 为 `33254920672`，Release ID 为 `378997476`；公开后 `published verification` run `33255604307` 成功，九项资产已按正式 tag URL、`releases/latest` 和 `latest.json` alias 完成匿名复验。

## 2026-08-30 v0.4.0 已发布事实

- `v0.4.0` 从 exact source `e462db05aa1f2608509cbf333d992bbd89be9a1d` 完成签名发布与公开复验；release run `33278181464`、published verification run `33278878143` 均成功，九项正式资产和稳定 `latest.json` alias 已核对。
- 该版本增加桌面端自动更新检查、最近尝试与成功状态的受限持久化、默认 12 小时且可配置 1–168 小时的检查间隔，以及设置侧栏/应用更新卡片视觉提醒；SQLite schema 升至 v10。
- 尚未从已安装的可信签名旧版本实际执行到 `v0.4.0` 的检查更新、下载验签、确认安装、重启与版本/活动页验证；`published publicly verified` 不能外推为 `updater E2E accepted`。

## 2026-08-30 v0.5.0-beta.1 community Pre-release 已发布事实

- 本版本从 exact source `dd370070ae58c3e70d61e95f3ad7b38278080a50` 和成功 CI run `33282176058` 发布，GitHub Release ID 为 `379137424`；它定版 Codex-only 自定义 Responses 供应商、本地 loopback 网关和“官方订阅”信息架构，SQLite schema 升至 v11。
- Release 明确标记为 `Unsigned Community Build` Pre-release，只包含版本化 unsigned DMG、SBOM ZIP、许可证 ZIP 和扁平 SHA-256 清单四项资产；没有 Developer ID、notarization、stapling、updater bundle 或 `latest.json`。
- 四项资产已经公开回下载并复验 tag/source、DMG、ZIP 和 SHA-256；稳定 `releases/latest` 与 updater alias 仍指向 `v0.4.0`。创建 tag 曾额外触发同 SHA 的 CI run `33282746660`，因此 `ci.yml` 从 `v0.5.0` 起将 push 限制到 `main`。
- 真实供应商、费用、手动配置往返、恢复直连和原生安装仍未 `accepted`；该 Pre-release 不得晋升、覆盖或复用为稳定版资产。

## 2026-08-30 v0.5.0 已发布事实

- `v0.5.0` 已从 exact source `fdc96c38de8d6d6cacc46a9b7fc70157d39439ee` 完成签名稳定发布。主线 CI run `33284042848`、受保护签名 release run `33284510727` 与公开后只读复验 run `33285144491` 均成功。
- GitHub Release ID 为 `379147257`，`draft=false`、`prerelease=false`；tag、Release、`releases/latest` 和 `latest.json` 均绑定同一 exact source。九项正式资产已按资产 ID 与正式 URL 回下载，并重跑 SHA-256、updater 验签、app/DMG Developer ID 签名、Hardened Runtime、双 notary `Accepted`、stapling、Gatekeeper、provenance 与 attestation 门禁。
- 该状态可报告为 `published publicly verified`，但不等于真实供应商或 updater 已经业务验收。真实 API-key Codex CLI Responses E2E、费用确认、手动配置往返/恢复直连，以及从可信旧签名版本完成检查、下载、验签、安装、重启到 `v0.5.0` 的 updater E2E 仍保持 `accepted=false`。
- `v0.5.0-beta.1` 仍是隔离的 unsigned community Pre-release；不得覆盖、晋升或复用它的 tag 与四项资产。

## 2026-08-30 v0.5.1 已发布事实

- `v0.5.1` 定版 Codex 配置管理页的紧凑卡片/新增编辑弹窗、桌面五列与窄屏响应式布局，并同步英文主 README、简体中文 README、五张脱敏截图、稳定 `releases/latest` 下载入口和上线前用户可见面同步门禁。
- 本版本不改变 SQLite schema、rollout/OTel/App Server 字段口径、OAuth/Keychain、网关协议、网络监听或权限边界；`docs/architecture.md`、`PRIVACY.md`、`docs/supply-chain.md` 与 `LICENSE` 因此记录为 `no change`。GitHub Description 已同步产品定位，Topics 已覆盖当前分类并记录为 `no change`。
- `v0.5.1` 已从 exact source `b933401353de46e39b1ea8e3f6ef1fb8065552b9` 完成签名稳定发布。主线 CI run `33293059226`、受保护签名 release run `33293480453` 与公开后只读复验 run `33294184588` 均成功；GitHub Release ID 为 `379187149`，`draft=false`、`prerelease=false`，tag、Release、`releases/latest` 与 `latest.json` 已绑定同一 source。
- 九项正式资产已按 draft asset ID 和公开 tag URL 回下载，完成 SHA-256、updater 签名、Developer ID、Hardened Runtime、secure timestamp、app/DMG 双 notary `Accepted`、stapling、Gatekeeper、provenance 与 attestation 复验。签名 job 在上传前已删除临时证书和 Keychain；该状态可报告为 `published publicly verified`。
- 真实 API-key 供应商 E2E、费用确认、手动配置往返/恢复直连，以及从可信旧签名版本安装更新到 `v0.5.1` 均不在本次授权范围内，继续保持 `accepted=false`。

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
8. transient failure 后可同时填写 exact `resume_run_id` 和 `resume_release_id`。只有 existing draft 的 source、release ID、sealed `RELEASE-INTENT.json`、build/sign evidence 和每个已上传资产 digest 全匹配时才续传/重验；任何冲突失败，不自动删除、覆盖或重新签名。已经删除的旧 draft `378845893` 不能 resume；run `33239136597` 又在创建 draft 前失败，因此现有双 ID 恢复入口也不适用，不能用手工 Release/API 写入绕过门禁。
9. 发布人在草稿页面和隔离机复核 source SHA、notary submission、Gatekeeper、哈希、SBOM、许可证、provenance 与 attestation。未获得发布授权时停在 draft；失败的 draft/tag 不自动处置。
10. 人工 Publish 后运行只读 `Verify existing macOS Release` 的 `published` 模式。它必须确认 `draft=false`、`prerelease=false`、tag 与 `releases/latest` 都绑定 exact Release/source，再匿名下载九个正式 tag URL 和 `/releases/latest/download/latest.json`，重跑第 7 步并保存 evidence。只有这样才能写成 `published publicly verified`。
11. 对 `v0.2.1` 另做干净 macOS 环境的 DMG quarantine、拖入 `/Applications`、启动与重启验收。由于 `v0.2.0` 无可信签名，真实 updater E2E 必须留到从 `v0.2.1` 升级到更高 SemVer；此前不能报告 `updater E2E accepted`。

私钥不做例行轮换。正常轮换必须先用旧 key 签名一个内嵌新公钥的桥接版本，确认旧客户端安装后再切换。如旧 key 已泄露，立即停止在线更新，不得再信任旧 key 签名的桥接包；改为人工分发嵌入新公钥的 Developer ID 签名且已公证版本。

## 回滚

保留 artifact、manifest、SBOM 和 commit provenance。发现问题时撤下 beta 下载入口并公告受影响版本，不要覆盖原 tag/asset/SHA。在线更新只接受更高 SemVer；回滚代码也必须发布为新的更高版本，例如将 `0.1.2` 的回退代码发布为 `0.1.3`。
