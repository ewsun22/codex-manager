import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { readCompleteJsonlLines } from "../src/core/jsonl-reader.ts";

test("emits only newline-terminated records and resumes by byte offset", async () => {
  const directory = await mkdtemp(join(tmpdir(), "codex-manager-jsonl-"));
  const filePath = join(directory, "fixture.jsonl");

  try {
    await writeFile(filePath, '{"value":"中文"}\n{"value":2}\n{"partial":', "utf8");

    const firstPass = [];
    for await (const entry of readCompleteJsonlLines(filePath)) {
      firstPass.push(entry);
    }

    assert.equal(firstPass.length, 2);
    assert.equal(JSON.parse(firstPass[0].line).value, "中文");
    assert.equal(JSON.parse(firstPass[1].line).value, 2);

    const checkpoint = firstPass[0].nextOffset;
    const resumed = [];
    for await (const entry of readCompleteJsonlLines(filePath, checkpoint)) {
      resumed.push(entry);
    }

    assert.equal(resumed.length, 1);
    assert.equal(JSON.parse(resumed[0].line).value, 2);
    assert.equal(resumed[0].startOffset, checkpoint);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("rejects invalid offsets", async () => {
  await assert.rejects(
    async () => {
      for await (const _entry of readCompleteJsonlLines("/tmp/missing", -1)) {
        // The range validation must happen before the file is opened.
      }
    },
    RangeError,
  );
});

test("bounds oversized lines and resumes at the next complete record", async () => {
  const directory = await mkdtemp(join(tmpdir(), "codex-manager-jsonl-cap-"));
  const filePath = join(directory, "fixture.jsonl");
  const diagnostics: Array<{ code: string; discardedBytes: number }> = [];

  try {
    await writeFile(filePath, "x".repeat(4097) + '\n{"ok":true}\n', "utf8");
    const entries = [];
    for await (const entry of readCompleteJsonlLines(filePath, 0, {
      maxLineBytes: 4096,
      onDiagnostic: (diagnostic) => diagnostics.push(diagnostic),
    })) {
      entries.push(entry);
    }

    assert.equal(diagnostics.length, 1);
    assert.equal(diagnostics[0].code, "line_too_large");
    assert.equal(diagnostics[0].discardedBytes, 4097);
    assert.equal(entries.length, 1);
    assert.deepEqual(JSON.parse(entries[0].line), { ok: true });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
