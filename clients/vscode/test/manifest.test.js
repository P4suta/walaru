"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const manifest = require("../package.json");

test("contributes live Java/Kotlin activation and resource-scoped settings", () => {
  assert.ok(manifest.contributes.viewsContainers.activitybar.some((item) => item.id === "walaru"));
  assert.equal(manifest.contributes.configuration.properties["walaru.binaryPath"].scope, "resource");
  assert.equal(
    manifest.contributes.configuration.properties["walaru.refreshIntervalSeconds"].scope,
    "resource",
  );
  assert.equal(manifest.contributes.testing, undefined);
  assert.ok(manifest.contributes.commands.some((item) => item.command === "walaru.fullVerify"));
  assert.ok(manifest.contributes.commands.some((item) => item.command === "walaru.runLive"));
  assert.equal(manifest.contributes.configuration.properties["walaru.live.mode"].scope, "resource");
  assert.equal(
    manifest.contributes.configuration.properties["walaru.live.debounceMilliseconds"].scope,
    "resource",
  );
  assert.ok(manifest.activationEvents.includes("onLanguage:java"));
  assert.ok(manifest.activationEvents.includes("onLanguage:kotlin"));
});
