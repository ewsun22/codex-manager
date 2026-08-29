#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { lstat, readFile, readlink, realpath } from "node:fs/promises";
import { isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EXIT_FINDING = 1;
const EXIT_SCANNER_ERROR = 2;
const MAX_FILE_BYTES = 64 * 1024 * 1024;

const privateKeyHeader = ["-----BEGIN ", "(?:(?:RSA|EC|OPENSSH|DSA|ENCRYPTED) )?", "PRIVATE KEY-----"].join("");
const minisignSecret = ["untrusted comment: minisign encrypted ", "secret key"].join("");
const releaseSecretNames = [
  "TAURI_SIGNING_PRIVATE_KEY",
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_TEAM_ID",
  "APPLE_SIGNING_IDENTITY",
];
const releaseSecretAlternation = releaseSecretNames.join("|");
const assignmentPattern = new RegExp(
  String.raw`(?:^|[\s"'{,;])(?:${releaseSecretAlternation})["']?\s*(?:=|:)\s*([^\r\n,;]+)`,
  "gim",
);
const blockAssignment = new RegExp(
  String.raw`(?:^|[\r\n])[ \t]*["']?(?:${releaseSecretAlternation})["']?[ \t]*:[ \t]*[>|][+-]?[0-9]?[ \t]*(?:#[^\r\n]*)?\r?\n[ \t]+\S`,
  "im",
);
const contentRules = [
  new RegExp(privateKeyHeader),
  new RegExp(minisignSecret, "i"),
  /AKIA[0-9A-Z]{16}/,
  /gh[pousr]_[A-Za-z0-9_]{30,}/,
  /github_pat_[A-Za-z0-9_]{40,}/,
  /sk-(?:proj-)?[A-Za-z0-9_-]{20,}/,
  /Authorization\s*:\s*Bearer\s+[A-Za-z0-9._~+\/-]{12,}=*/i,
];
const forbiddenPath = /(?:^|\/)(?:AuthKey_[^/]+\.p8|[^/]+\.(?:p12|pfx|cer|crt|key|mobileprovision))\s*$/i;

class ScannerError extends Error {}

function parseArgs(argv) {
  let root = process.cwd();
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] !== "--root" || index + 1 >= argv.length) {
      throw new ScannerError("仅支持 --root <git-worktree> 参数。");
    }
    root = argv[index + 1];
    index += 1;
  }
  return resolve(root);
}

function gitCandidates(root) {
  const result = spawnSync("git", ["-C", root, "ls-files", "-co", "--exclude-standard", "-z"], {
    encoding: null,
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "ignore"],
  });
  if (result.error?.code === "ENOENT") throw new ScannerError("缺少 git。");
  if (result.error || result.status !== 0 || !Buffer.isBuffer(result.stdout)) {
    throw new ScannerError("无法枚举 Git 工作树候选文件。");
  }
  return result.stdout.toString("utf8").split("\0").filter(Boolean);
}

function assertContained(root, candidate) {
  if (isAbsolute(candidate) || candidate.split(/[\\/]/).includes("..")) {
    throw new ScannerError("Git 返回了工作树之外的候选路径。");
  }
  const absolute = resolve(root, candidate);
  const rel = relative(root, absolute);
  if (rel === "" || rel.startsWith("..") || isAbsolute(rel)) {
    throw new ScannerError("Git 返回了工作树之外的候选路径。");
  }
  return absolute;
}

function containsKnownSecret(buffer) {
  const text = buffer.toString("latin1");
  if (contentRules.some((rule) => rule.test(text)) || blockAssignment.test(text)) return true;
  assignmentPattern.lastIndex = 0;
  for (const match of text.matchAll(assignmentPattern)) {
    let value = match[1].trim();
    if (!value.endsWith("}}")) value = value.replace(/}\s*$/, "").trim();
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1).trim();
    }
    if (/^\$\{\{\s*secrets\.[A-Z0-9_]+\s*}}$/.test(value)) continue;
    if (value.length >= 8) return true;
  }
  return false;
}

async function scan(root) {
  const canonicalRoot = await realpath(root).catch(() => {
    throw new ScannerError("扫描根目录不存在或不可访问。");
  });
  const findings = new Set();

  for (const candidate of gitCandidates(canonicalRoot)) {
    const normalized = candidate.replaceAll("\\", "/");
    if (forbiddenPath.test(normalized)) findings.add(candidate);

    const absolute = assertContained(canonicalRoot, candidate);
    let metadata;
    try {
      metadata = await lstat(absolute);
    } catch {
      throw new ScannerError(`无法读取候选文件元数据：${JSON.stringify(candidate)}`);
    }
    if (metadata.size > MAX_FILE_BYTES) {
      throw new ScannerError(`候选文件超过扫描上限：${JSON.stringify(candidate)}`);
    }

    let bytes;
    try {
      if (metadata.isSymbolicLink()) {
        bytes = Buffer.from(await readlink(absolute), "utf8");
      } else if (metadata.isFile()) {
        bytes = await readFile(absolute);
      } else {
        throw new ScannerError(`候选路径不是普通文件或符号链接：${JSON.stringify(candidate)}`);
      }
    } catch (error) {
      if (error instanceof ScannerError) throw error;
      throw new ScannerError(`无法完整读取候选文件：${JSON.stringify(candidate)}`);
    }
    if (containsKnownSecret(bytes)) findings.add(candidate);
  }

  return [...findings].sort((left, right) => left.localeCompare(right, "en"));
}

export { ScannerError, containsKnownSecret, forbiddenPath, scan };

async function main() {
  try {
    const findings = await scan(parseArgs(process.argv.slice(2)));
    if (findings.length > 0) {
      process.stderr.write("检测到疑似密钥或禁止进入仓库的证书文件，仅列出 JSON 转义后的文件名：\n");
      for (const finding of findings) process.stderr.write(`${JSON.stringify(finding)}\n`);
      process.exitCode = EXIT_FINDING;
      return;
    }
    process.stdout.write("已完整扫描候选文件，未检测到已知高风险密钥格式。\n");
  } catch (error) {
    const message = error instanceof ScannerError ? error.message : "发生未分类的扫描器错误。";
    process.stderr.write(`密钥扫描未完成：${message}\n`);
    process.exitCode = EXIT_SCANNER_ERROR;
  }
}

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) await main();
