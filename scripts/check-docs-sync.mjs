#!/usr/bin/env node
import { readdir, readFile, realpath, stat } from "node:fs/promises";
import { dirname, extname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const defaultRoot = fileURLToPath(new URL("..", import.meta.url));
const requiredSections = ["概览", "主要变化", "验证状态", "兼容性与限制", "隐私与安全", "发布状态", "完整性"];
const statusWords = ["implemented", "tested", "published", "observed", "accepted", "cleanup"];
const requiredDocuments = ["README.md", "README.zh-CN.md", "docs/capability-matrix.md", "docs/architecture.md", "docs/release.md", "docs/development-plan.md"];

function prose(markdown) {
  let fence = null;
  return markdown.replace(/<!--[\s\S]*?-->/g, (comment) => comment.replace(/[^\n]/g, " "))
    .split("\n").map((line) => {
      const marker = line.match(/^ {0,3}(`{3,}|~{3,})/);
      if (fence) {
        if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length) fence = null;
        return "";
      }
      if (marker) { fence = marker[1]; return ""; }
      return line;
    }).join("\n");
}

const referenceKey = (value) => value.replace(/\s+/g, " ").trim().toLowerCase();
const unescapeMarkdown = (value) => value.replace(/\\([!"#$%&'()*+,\-./:;<=>?@[\]^_`{|}~\\])/g, "$1").replaceAll("&amp;", "&");

function destination(value) {
  const trimmed = value.trim();
  if (trimmed.startsWith("<")) {
    const end = trimmed.indexOf(">");
    return end < 0 ? null : unescapeMarkdown(trimmed.slice(1, end));
  }
  return unescapeMarkdown(trimmed.match(/^(?:\\.|[^\s])*/)?.[0] ?? "");
}

/** Covers inline, reference and HTML links/images, including nested badge links. */
function links(markdown) {
  const text = prose(markdown).replace(/(`+)([\s\S]*?)\1/g, (code) => code.replace(/[^\n]/g, " "));
  const definitions = new Map();
  const body = text.replace(/^ {0,3}\[([^\]]+)\]:\s*(.+)$/gm, (line, label, value) => {
    definitions.set(referenceKey(label), destination(value));
    return line.replace(/[^\n]/g, " ");
  });
  const result = [];
  const stack = [];
  const add = (target, image, offset, label) => result.push({ target, image, label, line: body.slice(0, offset).split("\n").length });
  for (let index = 0; index < body.length; index += 1) {
    if (body[index] === "\\") { index += 1; continue; }
    if (body[index] === "[") { stack.push(index); continue; }
    if (body[index] !== "]" || !stack.length) continue;
    const start = stack.pop();
    const label = body.slice(start + 1, index);
    const image = start > 0 && body[start - 1] === "!";
    if (body[index + 1] === "(") {
      let end = index + 2;
      let depth = 1;
      let angle = false;
      for (; end < body.length; end += 1) {
        const character = body[end];
        if (character === "\\") { end += 1; continue; }
        if (character === "<") angle = true;
        if (character === ">") angle = false;
        if (!angle && character === "(") depth += 1;
        if (!angle && character === ")" && --depth === 0) break;
      }
      if (depth === 0) { add(destination(body.slice(index + 2, end)), image, start, label); index = end; }
      else add(null, image, start, label);
    } else if (body[index + 1] === "[") {
      const end = body.indexOf("]", index + 2);
      if (end < 0) { add(null, image, start, label); continue; }
      add(definitions.get(referenceKey(body.slice(index + 2, end) || label)) ?? null, image, start, label);
      index = end;
    } else if (definitions.has(referenceKey(label))) {
      add(definitions.get(referenceKey(label)), image, start, label);
    }
  }
  for (const match of body.matchAll(/<(a|img|source)\b[^>]*\b(href|src)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))[^>]*>/gi)) {
    add(unescapeMarkdown(match[3] ?? match[4] ?? match[5]), match[1].toLowerCase() !== "a", match.index, "");
  }
  for (const match of body.matchAll(/<(https?:\/\/[^\s<>]+)>/g)) add(unescapeMarkdown(match[1]), false, match.index, "");
  return result;
}

function anchors(markdown) {
  const body = prose(markdown);
  const found = new Set();
  const occurrences = new Map();
  for (const match of body.matchAll(/^ {0,3}#{1,6}\s+(.+?)\s*#*$/gm)) {
    const slug = match[1].toLowerCase().replace(/<[^>]*>/g, "").replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      .replace(/[^\p{L}\p{N}_\-\s]/gu, "").replace(/\s/g, "-");
    const count = occurrences.get(slug) ?? 0;
    occurrences.set(slug, count + 1);
    found.add(count ? `${slug}-${count}` : slug);
  }
  for (const match of body.matchAll(/<(?:a|[a-z][a-z0-9]*)\b[^>]*\b(?:id|name)=["']([^"']+)["'][^>]*>/gi)) found.add(match[1]);
  return found;
}

async function markdownFiles(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) files.push(...await markdownFiles(path));
    else if (entry.isFile() && entry.name.endsWith(".md")) files.push(path);
  }
  return files;
}

function platformFacts(content, file, errors) {
  const section = prose(content).match(/^##\s+(?:Supported platforms|支持平台|平台支持)\s*\n([\s\S]*?)(?=^##\s|$(?![\s\S]))/im)?.[1];
  if (!section) { errors.push(`${file}: 缺少 Supported platforms / 平台支持章节`); return null; }
  const facts = {};
  for (const platform of ["macOS", "Windows", "Linux"]) {
    const line = section.split("\n").find((value) => new RegExp(`\\b${platform}\\b`, "i").test(value));
    if (!line) { errors.push(`${file}: 平台章节缺少 ${platform}`); continue; }
    const unavailable = /unavailable|unsupported|not supported|不支持|未支持|尚不支持|暂不支持|不可用|未提供/i.test(line);
    const supported = /supported|支持|可用/i.test(line);
    const availability = unavailable ? "unavailable" : supported ? "supported" : null;
    if (!availability) errors.push(`${file}: ${platform} 缺少明确的 supported / unavailable 状态`);
    const versions = [...line.matchAll(new RegExp(`${platform}\\s*(\\d+(?:\\.\\d+){0,2})(\\+)?`, "gi"))]
      .map((match) => `${match[1].replace(/(?:\.0)+$/, "")}${match[2] ?? ""}`).sort();
    const architectures = [];
    if (/arm64|aarch64|apple silicon|apple\s*芯片/i.test(line)) architectures.push("arm64");
    if (/x86_64|\bx64\b|\bintel\b/i.test(line)) architectures.push("x86_64");
    facts[platform] = { availability, versions, architectures };
  }
  return facts;
}

function readmeFacts(content, parsedLinks, file, errors) {
  const versions = [...new Set([...prose(content).matchAll(/\bv?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)/g)].map((match) => match[1]))].sort();
  const downloads = [...new Set(parsedLinks.filter((link) => !link.image && /^https?:\/\//i.test(link.target ?? "")
    && (/\/releases(?:\/|$)/i.test(link.target) || /download|下载|release assets|校验和|安装包/i.test(link.label))).map((link) => link.target))].sort();
  if (!downloads.length) errors.push(`${file}: 缺少下载或 Release 入口`);
  return { versions, platforms: platformFacts(content, file, errors), downloads };
}

async function checkExternalLinks(urls, fetchImpl, timeoutMs, errors) {
  const pending = [...urls];
  await Promise.all(Array.from({ length: Math.min(4, pending.length) }, async () => {
    while (pending.length) {
      const url = pending.shift();
      try {
        const signal = AbortSignal.timeout(timeoutMs);
        let response = await fetchImpl(url, { method: "HEAD", signal, redirect: "follow", credentials: "omit" });
        if (response.status === 405 || response.status === 501) {
          await response.body?.cancel();
          response = await fetchImpl(url, { method: "GET", signal, redirect: "follow", credentials: "omit", headers: { Range: "bytes=0-0" } });
        }
        await response.body?.cancel();
        if (!response.ok) errors.push(`外部链接返回 HTTP ${response.status}: ${url}`);
      } catch { errors.push(`外部链接访问失败或超时: ${url}`); }
    }
  }));
}

export async function checkDocuments({ root = defaultRoot, checkExternal = false, fetchImpl = globalThis.fetch, externalTimeoutMs = 10_000 } = {}) {
  root = await realpath(resolve(root));
  const errors = [];
  const contents = new Map();
  for (const file of requiredDocuments) {
    try { contents.set(resolve(root, file), await readFile(resolve(root, file), "utf8")); }
    catch { errors.push(`缺少适用文档 ${file}`); }
  }
  let allDocs = [];
  try { allDocs = await markdownFiles(resolve(root, "docs")); }
  catch { errors.push("缺少 docs 文档目录"); }
  for (const file of ["SECURITY.md", "PRIVACY.md", "THIRD_PARTY_NOTICES.md"]) {
    try { contents.set(resolve(root, file), await readFile(resolve(root, file), "utf8")); }
    catch (error) { if (error.code !== "ENOENT") errors.push(`无法读取 ${file}`); }
  }
  for (const file of allDocs) if (!contents.has(file)) contents.set(file, await readFile(file, "utf8"));
  const notes = [...contents.keys()].filter((file) => /^docs\/release-notes\/(?:unreleased|v[^/]+)\.md$/.test(relative(root, file).replaceAll("\\", "/")));
  if (!notes.length) errors.push("docs/release-notes 中没有版本 note");
  for (const file of notes) {
    const content = prose(contents.get(file));
    const headings = [...content.matchAll(/^##\s+(.+)$/gm)].map((match) => match[1].trim());
    const label = relative(root, file);
    for (const required of requiredSections) if (!headings.includes(required)) errors.push(`${label}: 缺少 ## ${required}`);
    for (const status of statusWords) if (!new RegExp(`\\b${status}\\b`, "i").test(content)) errors.push(`${label}: 缺少状态词 ${status}`);
    if (/Codex Manager v[^\n]+ signed macOS arm64 release/i.test(content)) errors.push(`${label}: 不得使用固定的 updater notes 模板`);
  }

  const parsed = new Map();
  const externalUrls = new Set();
  let linkCount = 0;
  for (const [file, content] of contents) {
    const entries = links(content);
    parsed.set(file, entries);
    for (const link of entries) {
      linkCount += 1;
      const location = `${relative(root, file)}:${link.line}`;
      if (link.target === null) { errors.push(`${location}: 链接目标或引用定义缺失`); continue; }
      if (/^https?:\/\//i.test(link.target)) {
        try { const url = new URL(link.target); url.hash = ""; externalUrls.add(url.toString()); }
        catch { errors.push(`${location}: 无效外部 URL ${link.target}`); }
        continue;
      }
      if (/^(?:mailto|tel):/i.test(link.target) || link.image && /^data:image\//i.test(link.target)) continue;
      if (/^[a-z][a-z0-9+.-]*:/i.test(link.target) || link.target.startsWith("//")) { errors.push(`${location}: 不支持的文档链接 ${link.target}`); continue; }
      let target;
      let fragment;
      try {
        const [path, hash = ""] = link.target.split("#", 2);
        const pathname = decodeURIComponent(path.split("?", 1)[0]);
        fragment = decodeURIComponent(hash);
        target = pathname ? resolve(pathname.startsWith("/") ? root : dirname(file), pathname.replace(/^\//, "")) : file;
        const actual = await realpath(target);
        const withinRoot = relative(root, actual);
        if (withinRoot === ".." || withinRoot.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`) || isAbsolute(withinRoot)) { errors.push(`${location}: 相对链接指向仓库外 ${link.target}`); continue; }
        const info = await stat(actual);
        if (link.image && (!info.isFile() || info.size === 0)) errors.push(`${location}: 图片资产不是非空文件 ${link.target}`);
        if (fragment && extname(actual).toLowerCase() === ".md") {
          const targetContent = contents.get(actual) ?? await readFile(actual, "utf8");
          if (!anchors(targetContent).has(fragment)) errors.push(`${location}: Markdown 锚点不存在 ${link.target}`);
        }
      } catch { errors.push(`${location}: 相对链接或图片资产不存在 ${link.target}`); }
    }
  }
  const english = resolve(root, "README.md");
  const chinese = resolve(root, "README.zh-CN.md");
  if (contents.has(english) && contents.has(chinese)) {
    const left = readmeFacts(contents.get(english), parsed.get(english), "README.md", errors);
    const right = readmeFacts(contents.get(chinese), parsed.get(chinese), "README.zh-CN.md", errors);
    for (const [key, label] of [["versions", "版本"], ["platforms", "平台"], ["downloads", "下载入口"]]) {
      if (JSON.stringify(left[key]) !== JSON.stringify(right[key])) errors.push(`README.md / README.zh-CN.md: ${label}事实不同步（英文 ${JSON.stringify(left[key])}；中文 ${JSON.stringify(right[key])}）`);
    }
  }
  if (checkExternal) await checkExternalLinks(externalUrls, fetchImpl, externalTimeoutMs, errors);
  return { errors, documents: contents.size, notes: notes.length, links: linkCount, externalLinks: externalUrls.size };
}

async function main(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--root" && args[index + 1]) options.root = args[++index];
    else if (arg === "--check-external") options.checkExternal = true;
    else if (arg === "--external-timeout-ms" && /^\d+$/.test(args[index + 1] ?? "")) {
      options.externalTimeoutMs = Number(args[++index]);
      if (options.externalTimeoutMs < 1 || options.externalTimeoutMs > 60_000) throw new Error("外部链接超时须为 1–60000 ms");
    } else throw new Error(`不支持的参数 ${arg}`);
  }
  const result = await checkDocuments(options);
  if (result.errors.length) { console.error(result.errors.map((error) => `- ${error}`).join("\n")); process.exitCode = 1; }
  else console.log(`文档同步校验通过：${result.documents} 份文档，${result.notes} 个 release note，${result.links} 个链接；双语版本、平台和下载入口一致。外部 URL ${options.checkExternal ? "已检查" : "未联网检查（使用 --check-external 显式启用）"}。`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main(process.argv.slice(2)).catch((error) => { console.error(error.message); process.exitCode = 1; });
}
