"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const manifest = require("../package.json");

test("contributes an activity view and resource-scoped settings without Test Explorer claims", () => {
  assert.ok(manifest.contributes.viewsContainers.activitybar.some((item) => item.id === "walaru"));
  assert.equal(manifest.contributes.configuration.properties["walaru.binaryPath"].scope, "resource");
  assert.equal(
    manifest.contributes.configuration.properties["walaru.refreshIntervalSeconds"].scope,
    "resource",
  );
  assert.equal(manifest.contributes.testing, undefined);
  assert.ok(manifest.contributes.commands.some((item) => item.command === "walaru.fullVerify"));
});
