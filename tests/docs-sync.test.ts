import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import { checkDocuments } from "../scripts/check-docs-sync.mjs";

const english = `# Codex Manager

Current source: v0.6.5.
[中文](README.zh-CN.md)
[Download](https://github.com/fixture/docs/releases/latest)
![Dashboard](assets/screen.svg)

## Supported platforms

- Supported: macOS 13+, arm64.
- Windows: unavailable.
- Linux: unavailable.

## Guide

[Install](docs/guide.md#安装)
`;

const chinese = `# Codex Manager

当前源码：v0.6.5。
[English](README.md)
[下载](https://github.com/fixture/docs/releases/latest)
![总览](assets/screen.svg)

## 平台支持

- 已支持：macOS 13+，arm64。
- Windows：unavailable。
- Linux：unavailable。

## 使用说明

[安装](docs/guide.md#安装)
`;

const note = `# 未发布版本

## 概览
人工文档 fixture。
## 主要变化
implemented
## 验证状态
tested
## 兼容性与限制
observed
## 隐私与安全
accepted
## 发布状态
published
## 完整性
cleanup
`;

async function fixture(context: TestContext) {
  const root = await mkdtemp(join(tmpdir(), "codex-docs-sync-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, "docs/release-notes"), { recursive: true });
  await mkdir(join(root, "assets"));
  const files = {
    "README.md": english,
    "README.zh-CN.md": chinese,
    "docs/capability-matrix.md": "# 能力\n",
    "docs/architecture.md": "# 架构\n",
    "docs/release.md": "# 发布\n",
    "docs/development-plan.md": "# 开发\n",
    "docs/guide.md": "# 安装\n",
    "docs/release-notes/unreleased.md": note,
    "assets/screen.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>',
  };
  await Promise.all(Object.entries(files).map(([file, content]) => writeFile(join(root, file), content)));
  return root;
}

test("双语文档基线离线通过，不因下载入口存在而联网", async (context) => {
  const root = await fixture(context);
  let requested = false;
  const result = await checkDocuments({ root, fetchImpl: async () => { requested = true; throw new Error("network must not run"); } });
  assert.deepEqual(result.errors, []);
  assert.equal(result.notes, 1);
  assert.equal(requested, false);
});

test("缺少中文 README 必须失败", async (context) => {
  const root = await fixture(context);
  await rm(join(root, "README.zh-CN.md"));
  const result = await checkDocuments({ root });
  assert.ok(result.errors.some((message: string) => /缺少适用文档 README.zh-CN.md/.test(message)));
});

test("损坏的相对图片与空图片文件均失败，并指出引用位置", async (context) => {
  const root = await fixture(context);
  await writeFile(join(root, "README.md"), english.replace("assets/screen.svg", "assets/missing.svg"));
  await writeFile(join(root, "assets/screen.svg"), "");
  const result = await checkDocuments({ root });
  assert.ok(result.errors.some((message: string) => /README.md:\d+:.*assets\/missing.svg/.test(message)));
  assert.ok(result.errors.some((message: string) => /README.zh-CN.md:\d+: 图片资产不是非空文件/.test(message)));
});

test("两份 README 中的源码或运行时版本漂移会失败", async (context) => {
  const root = await fixture(context);
  await writeFile(join(root, "README.zh-CN.md"), chinese.replace("v0.6.5", "v0.6.4"));
  const result = await checkDocuments({ root });
  assert.ok(result.errors.some((message: string) => /版本事实不同步/.test(message)));
});

test("平台最低版本和支持状态漂移会失败", async (context) => {
  const root = await fixture(context);
  await writeFile(join(root, "README.zh-CN.md"), chinese.replace("macOS 13+", "macOS 14+").replace("Windows：unavailable", "Windows：已支持"));
  const result = await checkDocuments({ root });
  assert.ok(result.errors.some((message: string) => /平台事实不同步/.test(message)));
});

test("中文下载指向不同地址时失败，允许中英文使用各自截图", async (context) => {
  const root = await fixture(context);
  await writeFile(join(root, "assets/chinese.svg"), '<svg xmlns="http://www.w3.org/2000/svg"></svg>');
  await writeFile(join(root, "README.zh-CN.md"), chinese.replace("assets/screen.svg", "assets/chinese.svg"));
  assert.deepEqual((await checkDocuments({ root })).errors, []);
  const changed = (await readFile(join(root, "README.zh-CN.md"), "utf8")).replace("https://github.com/fixture/docs/releases/latest", "https://download.example.test/stale.dmg");
  await writeFile(join(root, "README.zh-CN.md"), changed);
  assert.ok((await checkDocuments({ root })).errors.some((message: string) => /下载入口事实不同步/.test(message)));
});

test("真实引用式、HTML、带空格路径和嵌套徽章链接会被检查，代码示例不误报", async (context) => {
  const root = await fixture(context);
  await writeFile(join(root, "assets/screen (wide).svg"), '<svg xmlns="http://www.w3.org/2000/svg"></svg>');
  await writeFile(join(root, "docs/guide.md"), `# 安装

![截图][shot]
[shot]: <../assets/screen (wide).svg> "wide screenshot"
[![badge](../assets/screen.svg)](../README.md#codex-manager)
<img src="../assets/screen%20(wide).svg" alt="screen">

\`[not a real link](missing-inline.md)\`
\`\`\`markdown
[example](missing-example.md)
\`\`\`
`);
  assert.deepEqual((await checkDocuments({ root })).errors, []);
  await rm(join(root, "assets/screen (wide).svg"));
  assert.ok((await checkDocuments({ root })).errors.some((message: string) => /docs\/guide.md:.*screen.*wide/.test(message)));
});

test("相对文档锚点和引用定义缺失会失败", async (context) => {
  const root = await fixture(context);
  await writeFile(join(root, "docs/guide.md"), "# 已改名\n\n[参考][missing-definition]\n");
  const errors = (await checkDocuments({ root })).errors;
  assert.ok(errors.some((message: string) => /Markdown 锚点不存在 docs\/guide.md#安装/.test(message)));
  assert.ok(errors.some((message: string) => /docs\/guide.md:.*引用定义缺失/.test(message)));
});

test("release note 的固定章节与状态门禁继续生效", async (context) => {
  const root = await fixture(context);
  await writeFile(join(root, "docs/release-notes/unreleased.md"), note.replace("## 隐私与安全\naccepted", "## Other\nnot yet"));
  const errors = (await checkDocuments({ root })).errors;
  assert.ok(errors.some((message: string) => /缺少 ## 隐私与安全/.test(message)));
  assert.ok(errors.some((message: string) => /缺少状态词 accepted/.test(message)));
});

test("显式外部检查才调用 HTTP；HEAD 不支持时使用受限 GET，无真实网络", async (context) => {
  const root = await fixture(context);
  const methods: string[] = [];
  const result = await checkDocuments({ root, checkExternal: true, fetchImpl: async (_url: unknown, init: RequestInit) => {
    methods.push(init.method ?? "");
    assert.equal(init.credentials, "omit");
    assert.ok(init.signal);
    if (init.method === "HEAD") return new Response(null, { status: 405 });
    assert.deepEqual(init.headers, { Range: "bytes=0-0" });
    return new Response(null, { status: 404 });
  } });
  assert.deepEqual(methods, ["HEAD", "GET"]);
  assert.ok(result.errors.some((message: string) => /外部链接返回 HTTP 404/.test(message)));
});
