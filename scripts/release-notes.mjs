#!/usr/bin/env node
import { readFile, writeFile } from "node:fs/promises";

const args = process.argv.slice(2);
const command = args[0];
const value = (flag) => {
  const index = args.indexOf(flag);
  if (index < 0 || !args[index + 1]) throw new Error(`缺少参数 ${flag}`);
  return args[index + 1];
};

if (command !== "summary") throw new Error("用法：node scripts/release-notes.mjs summary --input <release-note> --output <file>");
const input = await readFile(value("--input"), "utf8");
const sections = input.split(/^##\s+/m).slice(1).map((block) => {
  const newline = block.indexOf("\n");
  return { title: (newline < 0 ? block : block.slice(0, newline)).trim(), body: newline < 0 ? "" : block.slice(newline + 1) };
});
const selected = sections.filter(({ title }) => title === "概览" || title === "主要变化");
const lines = selected.flatMap(({ body }) => body.trim().split("\n").map((line) => line.trim()).filter((line) => line && !line.startsWith("#")));
const summary = lines.slice(0, 7).join(" ").replace(/\s+/g, " ").trim();
if (!summary) throw new Error("release note 的概览/主要变化没有可读内容");
await writeFile(value("--output"), summary.slice(0, 1200));
console.log(summary.slice(0, 1200));
