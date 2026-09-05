import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { formatCount, formatUsd } from "../src/app/format.ts";
import {
  detectDeviceLocale,
  hasUntranslatedChinese,
  readLocalePreference,
  resolveLocale,
  setCurrentLocale,
  t,
  tt,
  writeLocalePreference,
} from "../src/app/i18n-core.ts";

test("设备语言按 zh 前缀选择中文，其他语言选择英文", () => {
  assert.equal(detectDeviceLocale(["zh-TW", "en-US"]), "zh-CN");
  assert.equal(detectDeviceLocale(["zh-HK"]), "zh-CN");
  assert.equal(detectDeviceLocale(["en-US", "zh-CN"]), "en-US");
  assert.equal(detectDeviceLocale(["ja-JP"]), "en-US");
  assert.equal(detectDeviceLocale([]), "en-US");
});

test("手动语言偏好优先于设备语言，并支持恢复自动", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
  };
  assert.equal(readLocalePreference(storage), "auto");
  writeLocalePreference("zh-CN", storage);
  assert.equal(readLocalePreference(storage), "zh-CN");
  assert.equal(resolveLocale("zh-CN", ["en-US"]), "zh-CN");
  writeLocalePreference("auto", storage);
  assert.equal(readLocalePreference(storage), "auto");
  assert.equal(resolveLocale("auto", ["en-US"]), "en-US");
  assert.equal(readLocalePreference({ getItem: () => { throw new Error("storage disabled"); } }), "auto");
});

test("源码级翻译保留动态项目数据，并切换 locale 格式", () => {
  setCurrentLocale("en-US");
  assert.equal(t("设置"), "Settings");
  assert.equal(tt`查看 ${"中文项目"} 的详情`, "View 中文项目 details");
  assert.equal(formatCount(12_345), "12,345");
  assert.equal(formatUsd(12.5), "$12.50");
  setCurrentLocale("zh-CN");
  assert.equal(t("设置"), "设置");
  assert.equal(formatUsd(12.5), "US$12.50");
});

test("所有主要界面源码文案都有英文翻译，且不再扫描 DOM", async () => {
  const source = await readFile(new URL("../src/app/i18n.tsx", import.meta.url), "utf8");
  const core = await readFile(new URL("../src/app/i18n-core.ts", import.meta.url), "utf8");
  const uiSources = await Promise.all([
    readFile(new URL("../src/App.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/codex-config.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/codex-gateway.tsx", import.meta.url), "utf8"),
    readFile(new URL("../src/app/operations.ts", import.meta.url), "utf8"),
    readFile(new URL("../src/app/confirmation.tsx", import.meta.url), "utf8"),
  ]);
  assert.match(core, /codex-manager\.locale\.v1/);
  assert.match(source, /aria-pressed/);
  assert.match(source, /aria-label=\{t\("跟随设备语言"\)\}/);
  assert.match(core, /document\.documentElement\.lang/);
  assert.doesNotMatch(source, /MutationObserver|TreeWalker|translateDom/);

  setCurrentLocale("en-US");
  const missing: string[] = [];
  for (const uiSource of uiSources) {
    const withoutCalls = uiSource
      .replace(/\bt\(("(?:\\.|[^"\\])*")\)/g, "")
      .replace(/\btt`((?:\\.|[^`\\])*)`/gs, "");
    for (const line of withoutCalls.split("\n")) assert.doesNotMatch(line, />[^<{]*[\u3400-\u9fff]/u);
    for (const match of uiSource.matchAll(/\bt\(("(?:\\.|[^"\\])*")\)/g)) {
      const sourceText = JSON.parse(match[1]) as string;
      if (hasUntranslatedChinese(t(sourceText))) missing.push(sourceText);
    }
    for (const match of uiSource.matchAll(/\btt`((?:\\.|[^`\\])*)`/gs)) {
      for (const rawSegment of match[1].split(/\$\{[\s\S]*?\}/g)) {
        const sourceText = rawSegment.replace(/\\n/g, "\n");
        if (hasUntranslatedChinese(t(sourceText))) missing.push(sourceText);
      }
    }
  }
  setCurrentLocale("zh-CN");
  assert.deepEqual([...new Set(missing)], []);
});
