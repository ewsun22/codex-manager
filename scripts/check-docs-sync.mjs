#!/usr/bin/env node
import { readdir, readFile } from "node:fs/promises";

const root = new URL("..", import.meta.url);
const requiredSections = ["概览", "主要变化", "验证状态", "兼容性与限制", "隐私与安全", "发布状态", "完整性"];
const statusWords = ["implemented", "tested", "published", "observed", "accepted", "cleanup"];

function sections(markdown) {
  return [...markdown.matchAll(/^##\s+(.+)$/gm)].map((match) => match[1].trim());
}

const notesDirectory = new URL("docs/release-notes/", root);
const entries = (await readdir(notesDirectory))
  .filter((name) => name === "unreleased.md" || /^v.+\.md$/.test(name))
  .sort();
if (entries.length === 0) throw new Error("docs/release-notes 中没有版本 note");

const errors = [];
for (const name of entries) {
  const path = new URL(name, notesDirectory);
  const content = await readFile(path, "utf8");
  const headings = sections(content);
  for (const required of requiredSections) {
    if (!headings.includes(required)) errors.push(`${name}: 缺少 ## ${required}`);
  }
  for (const status of statusWords) {
    if (!new RegExp(`\\b${status}\\b`, "i").test(content)) errors.push(`${name}: 缺少状态词 ${status}`);
  }
  if (/Codex Manager v[^\n]+ signed macOS arm64 release/i.test(content)) {
    errors.push(`${name}: 不得使用固定的 updater notes 模板`);
  }
}

for (const file of ["README.md", "docs/capability-matrix.md", "docs/architecture.md", "docs/release.md", "docs/development-plan.md"]) {
  try { await readFile(new URL(file, root), "utf8"); }
  catch { errors.push(`缺少适用文档 ${file}`); }
}

if (errors.length) {
  console.error(errors.map((error) => `- ${error}`).join("\n"));
  process.exitCode = 1;
} else {
  console.log(`文档同步校验通过：${entries.length} 个 release note，固定章节与状态边界完整。`);
}
