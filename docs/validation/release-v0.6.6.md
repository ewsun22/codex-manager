# v0.6.6 发布准备与用户可见面同步

日期：2026-09-06。用户已在真实 OAuth 未验收和旧版 updater 跳过的边界明确后授权“提交，推送，releases”。本切片将 `45ba523` 已通过验证的功能定为 v0.6.6；具体发布结果以 [发布记录](../release.md) 的 exact source、workflow run 和公开资产证据为准。

## 定版与验证

- `package.json`、npm lock 根版本、Cargo workspace 与三个本地 crate lock 项、Tauri 配置及浏览器 demo 版本同步为 `0.6.6`。没有修改业务逻辑、权限或内核版本。
- `npm run check`：类型检查、125 项 Node 测试及 fixture probe 通过；`npm run build` 通过。
- 文档检查通过 32 份文档、15 个版本说明、132 个链接及外部 URL；双语版本/平台/下载事实一致。两份 README、版本说明和本文经 GitHub GFM 渲染核对。版本、secret、差异检查和发布状态机 dry-run 通过；updater 摘要为 774 字符，双语验收边界完整。
- S1–S6 的 Rust、扫描基准、5 项浏览器确认与真实 CLIProxyAPI 空池证据沿用 [S5–S6 记录](reliability-s5-s6.md)，不是再次发起真实 OAuth。
- 定版后的隔离真实 Tauri 桌面再次通过 8 项检查，并经实际按钮允许关闭，native Destroyed 为 true。见 [v0.6.6 桌面结果](desktop-qa-v0.6.6.json) 与 [真实截图](../../output/playwright/final-release-v0.6.6-confirmation.png)。QA 进程和开发服务器正常退出。
- 默认产品构建不启用 `desktop-qa`；Release workflow 无 `--features desktop-qa` 或 `--all-features`。只有显式 debug QA 构建会启用人工数据入口，release + QA 的编译拒绝门保持不变。

## 用户可见面同步表

| 用户可见面 | 本次处理与依据 |
| --- | --- |
| README 英文 / 简体中文 | 同步 v0.6.6 功能、版本说明链接、真实确认框截图、OAuth 未验收与 updater skipped 状态 |
| 应用版本与 demo | 均改为 0.6.6；源模型名、项目名、人工 fixture 版本不批量替换 |
| 截图 / GIF | 新增 v0.6.6 debug 人工数据的真实确认框截图，未编辑像素；既有总览、活动、账户与代理截图 no change，因本次没有改其布局；此前验证截图保留原版本历史事实；无 GIF |
| 架构 / 能力 / 计划 | 定版当前可靠性行为，清理当前功能中过时的“未发布”表述；历史 tag 与验证记录不改写 |
| 安全 / 隐私 / 供应链 | 同步保留期、空池观测已完成及本次限定发布例外；真实 OAuth 和 5xx 未验收；签名/公证/公开校验门不变 |
| Release / latest.json.notes | 使用同一 `v0.6.6.md` 完整说明和脚本短摘要，不手写不同版本摘要 |
| 下载 / Quick Start / 徽章 | no change：均使用动态 latest 稳定入口，macOS 13+ arm64 支持范围未变；公开后校验 latest 指向新版本 |
| GitHub Description / Topics | 已只读核对，no change：仍准确描述本地 Codex 管理器及现有功能；无品牌或平台扩展 |
| GitHub homepage / 网站 / 商店 | no change：homepage 为空，本次没有网站、商店或其他部署入口 |
| LICENSE / NOTICE / 第三方许可 | no change：本次定版不增依赖、不改许可证，SBOM/许可证资产仍由 exact source 重新生成 |
| 回退与清理 | 保留旧 tag 与资产；发布后问题通过更高 SemVer 修复，不能覆盖 v0.6.6；原有 `design-qa.md` 不纳入提交 |

## 发布顺序与限制

最终源码经过版本、文档、链接/图片、Markdown 渲染和 secret 检查后提交并推送。对 exact SHA 执行主线 CI 和签名发布 workflow；受保护签名批准前核对 source、版本及无秘密 build 结果。生成九项资产并完成按 ID 草稿回下载、签名/公证/Gatekeeper、updater、provenance/attestation 和清理门禁后，才公开 Release。随后运行独立 `published` 只读复验，确认正式 tag URL 与 latest alias。

`implemented` / `tested`：定版与本地检查完成。`published`：本文在源码定版时不提前宣称发布完成。`observed`：人工桌面和空池内核已有证据。`accepted`：真实 OAuth 业务仍未验收；旧版 updater 为 `skipped by user`。后续发布结果单独记录，不改写 tag 时状态。

`deployment_decision=project-authorized`：本次用户明确授权提交、推送和 Release，覆盖 v0.6.6 的既有签名稳定发布流程；不扩展为真实账户调用、配置切换或本机安装升级。
