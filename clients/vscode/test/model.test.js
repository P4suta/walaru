"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  commandArguments,
  executionAllowed,
  formatEnvelope,
  retainWorkspaceModels,
  statusSummary,
  workspaceModel,
} = require("../model");

function folder(name, fsPath) {
  return { name, uri: { fsPath, toString: () => `file://${fsPath}` } };
}

function envelope(data, overrides = {}) {
  return {
    schemaVersion: "1",
    workspaceId: "ws-test",
    revision: "rev-test",
    sessionId: "session-test",
    runId: null,
    status: "ok",
    data,
    diagnostics: [],
    capabilities: { backend: "none", completeness: "unsupported", supported: [], unavailable: {} },
    nextActions: [],
    page: null,
    ...overrides,
  };
}

test("builds independent multi-root models with failures first", () => {
  const alpha = workspaceModel(
    folder("alpha", "/work/alpha"),
    envelope({ running: true }),
    envelope({
      tests: [
        { id: "z#passes", displayName: "passes", module: ":z", lastStatus: "passed", lastFailureId: null },
        { id: "a#fails", displayName: "fails", module: ":a", lastStatus: "failed", lastFailureId: "failure-1" },
      ],
    }),
  );
  const beta = workspaceModel(
    folder("beta", "/work/beta"),
    envelope({ running: true }),
    envelope({ tests: [] }),
  );
  assert.equal(alpha.folder.name, "alpha");
  assert.equal(alpha.tests[0].id, "a#fails");
  assert.equal(alpha.tests[0].lastFailureId, "failure-1");
  assert.equal(beta.folder.name, "beta");
  assert.equal(beta.tests.length, 0);

  const models = new Map([
    [alpha.folder.uri.toString(), alpha],
    [beta.folder.uri.toString(), beta],
  ]);
  retainWorkspaceModels(models, [beta.folder]);
  assert.deepEqual(Array.from(models.keys()), [beta.folder.uri.toString()]);
});

test("aggregates status and represents workspace trust", () => {
  const current = workspaceModel(
    folder("alpha", "/work/alpha"),
    envelope({ running: true }),
    envelope({
      tests: [
        { id: "a#fails", module: ":", lastStatus: "failed", lastFailureId: "failure-1" },
      ],
    }),
  );
  const summary = statusSummary([current], true);
  assert.equal(summary.failures, 1);
  assert.match(summary.text, /1 failed \/ 1/);
  assert.equal(executionAllowed(false), false);
  assert.equal(executionAllowed(true), true);
  assert.match(statusSummary([], false).text, /untrusted/);
});

test("builds exact shell-free command argv", () => {
  const failed = { id: "demo.Test#fails", lastFailureId: "failure-7" };
  assert.deepEqual(commandArguments("failure", failed), ["failure", "failure-7"]);
  assert.deepEqual(commandArguments("trace", failed), ["trace", "demo.Test#fails"]);
  assert.deepEqual(commandArguments("record", failed), ["record", "demo.Test#fails"]);
  assert.deepEqual(commandArguments("verify"), ["verify"]);
  assert.deepEqual(commandArguments("fullVerify"), ["verify", "--full"]);
  assert.throws(
    () => commandArguments("failure", { id: "demo.Test#passes" }),
    /no recorded failure/,
  );
});

test("formats human output instead of dumping JSON", () => {
  const output = formatEnvelope(
    "failure",
    envelope({
      failure: {
        id: "failure-1",
        testId: "demo.Test#fails",
        exceptionType: "java.lang.AssertionError",
        message: "expected 1",
        frames: ["Test.kt:9"],
      },
      analysis: {
        summary: "Assertion failed: expected 1, observed 2.",
        likelyCause: "Captured `actual` with value 2 immediately preceded the failure.",
        focus: { path: "src/test/kotlin/Test.kt", line: 9 },
        evidence: [{ label: "Captured `actual`", value: 2 }],
        suggestions: ["Inspect the focused source line."],
      },
    }),
  );
  assert.match(output, /java\.lang\.AssertionError/);
  assert.match(output, /expected 1/);
  assert.match(output, /Why this probably failed/);
  assert.match(output, /Captured `actual`/);
  assert.doesNotMatch(output, /\"failure\"/);
});
