# 文档同步检查

`npm run docs:check` 默认只读取本地仓库，不访问互联网。它用于提交和发布前的自动校验，也可在日常开发中运行。此检查补充人工用户可见面审查；通过不代表已发布、截图已与真实桌面一致或端到端业务已验收。

## 本地检查范围

- 必须同时存在英文 `README.md`、中文 `README.zh-CN.md`，以及能力矩阵、架构、发布运行手册和开发计划。
- 扫描两份 README、已有的安全/隐私/第三方声明，以及 `docs` 中的全部 Markdown 文档。检查 Markdown 行内链接、引用式链接、嵌套徽章、HTML `href` / `src` 和图片目标；代码围栏、行内代码与 HTML 注释中的示例不当作实际链接。
- 相对目标必须存在并位于仓库内；图片必须是非空文件。支持带空格的尖括号路径、百分号编码路径和当前目录/父目录路径。Markdown 标题锚点和显式 HTML `id` / `name` 也会校验。图片内容的视觉真实性、截图新旧和浏览器渲染仍由人工检查。
- 两份 README 中出现的 SemVer 版本集合必须相同，包括产品、运行时及历史版本引用。检查只比较文档事实，不把源码版本当作线上已发布版本。
- 平台章节使用 `## Supported platforms` 和 `## 平台支持`（也接受 `## 支持平台`），逐行明确 macOS、Windows 和 Linux 的 `supported` / `unavailable` 状态或中文等价表述。自动比较支持状态、所写的最低版本与架构；`arm64`、`aarch64`、Apple silicon / Apple 芯片归为同一架构。
- 下载/Release 链接集合必须一致；允许两种语言使用各自的截图路径和译文。不同下载 CDN、tag 或版本资产地址会导致失败。
- 版本 release note 和 `unreleased.md` 继续要求“概览”“主要变化”“验证状态”“兼容性与限制”“隐私与安全”“发布状态”“完整性”七个章节，并分别保留 `implemented`、`tested`、`published`、`observed`、`accepted`、`cleanup` 状态词；禁止旧固定 updater notes 模板。

## 可选外部链接检查

准备推送或发布时，可显式执行：

```bash
npm run docs:check -- --check-external
```

外部 HTTP(S) URL 去重后以最多四个并发检查。先发送 HEAD；服务器返回 405 或 501 时退回带 `Range: bytes=0-0` 的 GET，并取消响应正文。默认每个 URL 超时 10 秒，不携带账号凭据，也不进行登录或远端写入。可用 `--external-timeout-ms 15000` 调整，范围为 1–60000 毫秒。

网络错误、非成功 HTTP 状态和超时均导致检查失败。404 需要修正链接；403、429 或暂时不可达可能需要人工核实，不能据此宣称目标永久失效。跳转后的网络可访问性不证明下载资产的签名、哈希、版本或平台正确，这些仍由发布流程验证。

## 验证与限制

`tests/docs-sync.test.ts` 使用临时人工文档和图片 fixture，覆盖中文 README 缺失、图片断链、版本/平台/下载漂移、引用式链接和锚点，以及原 release note 门禁。外部检查测试只注入人工 HTTP 响应，因此 `npm test` 不依赖互联网。

脚本支持 `--root <目录>`，用于复核其他 checkout 或运行人工 fixture。解析器覆盖本项目使用的 Markdown 链接和 ATX 标题语法；它不是完整 Markdown 渲染器。平台行的自然语言含义、译文功能事实、图片视觉质量、HTML 动态内容、CSS 引用、`srcset` 和线上 Repository 元数据仍需人工审阅。
