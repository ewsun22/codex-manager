import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const workflowDirectory = fileURLToPath(new URL("../.github/workflows/", import.meta.url));

function shellBlocks(source: string): string[] {
  const lines = source.split("\n");
  const blocks: string[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    const marker = /^(\s*)run:\s*\|[-+]?\s*$/.exec(lines[index]);
    if (!marker) continue;
    const markerIndent = marker[1].length;
    const contentIndent = markerIndent + 2;
    const block: string[] = [];
    for (index += 1; index < lines.length; index += 1) {
      const line = lines[index];
      const indentation = /^(\s*)/.exec(line)?.[1].length ?? 0;
      if (line.length > 0 && indentation <= markerIndent) {
        index -= 1;
        break;
      }
      block.push(line.length === 0 ? "" : line.slice(contentIndent));
    }
    blocks.push(`${block.join("\n")}\n`);
  }
  return blocks;
}

test("所有 GitHub Actions run block 都通过 Bash 语法检查", () => {
  const files = readdirSync(workflowDirectory).filter((name) => name.endsWith(".yml")).sort();
  let count = 0;
  for (const file of files) {
    const source = readFileSync(join(workflowDirectory, file), "utf8");
    for (const [index, block] of shellBlocks(source).entries()) {
      const result = spawnSync("/bin/bash", ["-n"], { input: block, encoding: "utf8" });
      assert.equal(result.status, 0, `${file} run block ${index + 1}: ${result.stderr}`);
      count += 1;
    }
  }
  assert.ok(count >= 10, "expected release and CI workflow shell blocks");
});

test("所有第三方 GitHub Action 都固定为完整 commit SHA", () => {
  const files = readdirSync(workflowDirectory).filter((name) => name.endsWith(".yml")).sort();
  for (const file of files) {
    const source = readFileSync(join(workflowDirectory, file), "utf8");
    const uses = [...source.matchAll(/^\s*uses:\s*([^\s#]+)/gm)].map((match) => match[1]);
    assert.ok(uses.length > 0, `${file} should contain pinned actions`);
    for (const action of uses) assert.match(action, /^[^@]+@[a-f0-9]{40}$/, `${file}: ${action}`);
  }
});
