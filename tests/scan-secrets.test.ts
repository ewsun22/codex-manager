import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const scanner = fileURLToPath(new URL("../scripts/scan-secrets.sh", import.meta.url));

function findExecutable(name: string): string {
  for (const directory of (process.env.PATH ?? "").split(delimiter)) {
    const candidate = join(directory, name);
    if (existsSync(candidate)) return realpathSync(candidate);
  }
  throw new Error(`missing test prerequisite: ${name}`);
}

function createFixture(): string {
  const root = mkdtempSync(join(tmpdir(), "codex-manager-secret-scan-"));
  const result = spawnSync(findExecutable("git"), ["init", "-q", root], {
    env: { ...process.env, DEVELOPER_DIR: "/Library/Developer/CommandLineTools" },
  });
  assert.equal(result.status, 0);
  return root;
}

function runScanner(root: string, path = process.env.PATH ?? "") {
  return spawnSync("/bin/bash", [scanner, "--root", root], {
    encoding: "utf8",
    env: { ...process.env, DEVELOPER_DIR: "/Library/Developer/CommandLineTools", PATH: path },
  });
}

test("secret scanner 完整扫描 clean fixture", () => {
  const root = createFixture();
  try {
    writeFileSync(join(root, "clean.txt"), "synthetic release fixture\n");
    const result = runScanner(root);
    assert.equal(result.status, 0);
    assert.match(result.stdout, /已完整扫描/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("secret scanner 在无 rg 的 PATH 中仍实际扫描并拒绝人工 marker", () => {
  const root = createFixture();
  const bin = mkdtempSync(join(tmpdir(), "codex-manager-secret-bin-"));
  const marker = ["AKIA", "A".repeat(16)].join("");
  try {
    symlinkSync(process.execPath, join(bin, "node"));
    symlinkSync(findExecutable("git"), join(bin, "git"));
    writeFileSync(join(root, "candidate.txt"), marker);
    const result = runScanner(root, bin);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /"candidate\.txt"/);
    assert.equal(result.stderr.includes(marker), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
    rmSync(bin, { recursive: true, force: true });
  }
});

test("secret scanner 扫描 NUL 后的二进制 marker", () => {
  const root = createFixture();
  const marker = ["ghp_", "B".repeat(32)].join("");
  try {
    writeFileSync(join(root, "binary.dat"), Buffer.concat([Buffer.from([0, 1, 2, 0]), Buffer.from(marker)]));
    const result = runScanner(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /"binary\.dat"/);
    assert.equal(result.stderr.includes(marker), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("secret scanner 拒绝 Apple 与 Tauri 发布变量的人工 literal assignment", () => {
  const root = createFixture();
  const appleAssignment = ["APPLE", "_ID=", "synthetic@example.invalid"].join("");
  const updaterAssignment = ["TAURI_SIGNING_PRIVATE_KEY", "_PASSWORD=", "synthetic-password-material"].join("");
  try {
    writeFileSync(join(root, "release.env"), `${appleAssignment}\n${updaterAssignment}\n`);
    const result = runScanner(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /"release\.env"/);
    assert.equal(result.stderr.includes("synthetic@example.invalid"), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("secret scanner 拒绝危险后缀及特殊文件名并转义日志", () => {
  const root = createFixture();
  try {
    writeFileSync(join(root, "-certificate.p12"), "synthetic bytes");
    writeFileSync(join(root, "line\nbreak.key"), "synthetic bytes");
    const result = runScanner(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /"-certificate\.p12"/);
    assert.match(result.stderr, /"line\\nbreak\.key"/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("secret scanner 拒绝 encrypted PKCS8、YAML block、完整 Bearer 字符和伪装证书后缀", () => {
  const root = createFixture();
  const encryptedHeader = ["-----BEGIN ", "ENCRYPTED ", "PRIVATE KEY-----"].join("");
  const yamlBlock = ["APPLE", "_CERTIFICATE: >-\n  synthetic-base64-material"].join("");
  const bearer = ["Authorization: Bearer abc+", "defghijklmnopqrstuvwxyz"].join("");
  try {
    writeFileSync(join(root, "encrypted.txt"), `${encryptedHeader}\nsynthetic-material`);
    writeFileSync(join(root, "block.yml"), yamlBlock);
    writeFileSync(join(root, "bearer.txt"), bearer);
    writeFileSync(join(root, "candidate.p12 "), "synthetic bytes");
    const result = runScanner(root);
    assert.equal(result.status, 1);
    for (const name of ["encrypted.txt", "block.yml", "bearer.txt", "candidate.p12 "]) {
      assert.ok(result.stderr.includes(JSON.stringify(name)), `missing finding for ${JSON.stringify(name)}`);
    }
    assert.equal(result.stderr.includes("synthetic-material"), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("secret scanner 只豁免精确 GitHub secret reference，不豁免普通变量占位", () => {
  const cleanRoot = createFixture();
  const findingRoot = createFixture();
  const name = ["APPLE", "_ID"].join("");
  try {
    writeFileSync(join(cleanRoot, "workflow.yml"), `${name}: \${{ secrets.${name} }}\n`);
    assert.equal(runScanner(cleanRoot).status, 0);
    writeFileSync(join(findingRoot, "placeholder.env"), `${name}=$${name}\n`);
    const finding = runScanner(findingRoot);
    assert.equal(finding.status, 1);
    assert.match(finding.stderr, /"placeholder\.env"/);
  } finally {
    rmSync(cleanRoot, { recursive: true, force: true });
    rmSync(findingRoot, { recursive: true, force: true });
  }
});

test("secret scanner 缺少 Node 时 fail closed", () => {
  const root = createFixture();
  const emptyPath = mkdtempSync(join(tmpdir(), "codex-manager-empty-path-"));
  try {
    const result = runScanner(root, emptyPath);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /缺少 Node\.js/);
  } finally {
    rmSync(root, { recursive: true, force: true });
    rmSync(emptyPath, { recursive: true, force: true });
  }
});

test("secret scanner 缺少 Git 时以 scanner error fail closed", () => {
  const root = createFixture();
  const nodeOnlyPath = mkdtempSync(join(tmpdir(), "codex-manager-node-only-path-"));
  try {
    symlinkSync(process.execPath, join(nodeOnlyPath, "node"));
    const result = runScanner(root, nodeOnlyPath);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /缺少 git/);
  } finally {
    rmSync(root, { recursive: true, force: true });
    rmSync(nodeOnlyPath, { recursive: true, force: true });
  }
});
