import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const appSource = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const backendSource = readFileSync(new URL("../src-tauri/src/main.rs", import.meta.url), "utf8");

test("activity uses bootstrap data before polling every five seconds", () => {
  assert.match(appSource, /nextQueryKey === observedQueryKey\.current\) return;/);
  assert.match(appSource, /setInterval\(pollRevision, 5_000\)/);
  assert.match(appSource, /onLoad\(\{ revisionOnly: true \}\)/);
  assert.match(appSource, /refreshOnly: !current\.countTotal/);
  assert.match(appSource, /matchesActiveQuery && active\.countTotal/);
});

test("background and manual activity refreshes avoid a full bootstrap", () => {
  assert.match(
    appSource,
    /activeViewRef\.current === "activity"[\s\S]*COMMANDS\.rescan[\s\S]*setActivityRefreshRevision/,
  );
  assert.match(
    appSource,
    /listen\("local-data-refreshed"[\s\S]*activeViewRef\.current === "activity"[\s\S]*setActivityRefreshRevision/,
  );
});

test("startup defers dashboard aggregation and activity count", () => {
  assert.match(backendSource, /summary: None,[\s\S]*refresh_only: Some\(true\)/);
  assert.match(appSource, /COMMANDS\.getDashboard[\s\S]*activity: current\.activity\.total === null/);
  assert.match(appSource, /current\.activity\.revision === summary\.activityRevision/);
  assert.match(backendSource, /activity_revision: String/);
  assert.match(backendSource, /store\.dashboard_snapshot\(\)/);
  assert.match(backendSource, /\.activity_snapshot\(&filter, !refresh_only\)/);
});
