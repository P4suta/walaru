"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  ignoredWorkspacePath,
  LiveScheduler,
  livePresentation,
  normalizeRelativePath,
  overlayManifest,
  overlayVersionsMatch,
} = require("../live");

test("ignores generated trees so the private mirror cannot retrigger live verification", () => {
  assert.equal(ignoredWorkspacePath(".gradle/walaru/live/vscode/mirror/src/App.java"), true);
  assert.equal(ignoredWorkspacePath("module/build/generated/src/App.java"), true);
  assert.equal(ignoredWorkspacePath("module/src/main/java/App.java"), false);
  assert.equal(ignoredWorkspacePath("module/src/main/java/example/target/App.java"), false);
});

test("builds a bounded canonical unsaved-document manifest", () => {
  assert.deepEqual(overlayManifest("vscode", []), {
    schemaVersion: 1,
    sessionId: "vscode",
    documents: [],
  });
  assert.deepEqual(
    overlayManifest("vscode", [
      { path: "src/main/java/demo/App.java", version: 7, content: "class App {}" },
    ]),
    {
      schemaVersion: 1,
      sessionId: "vscode",
      documents: [
        { path: "src/main/java/demo/App.java", version: 7, content: "class App {}" },
      ],
    },
  );
  assert.equal(normalizeRelativePath("module/src/Test.kt"), "module/src/Test.kt");
  assert.throws(() => normalizeRelativePath("../secret"), /Unsafe/);
  assert.throws(() => normalizeRelativePath("src\\Secret.java"), /workspace-relative/);
  assert.throws(() => normalizeRelativePath("src/Secret\n.java"), /workspace-relative/);
  assert.throws(
    () => overlayManifest("vscode", [{ path: "src/A.java", version: 1, content: "x".repeat(1024 * 1024 + 1) }]),
    /exceeds/,
  );
  assert.throws(
    () => overlayManifest("vscode", [
      { path: "src/A.java", version: 1, content: "class A {}" },
      { path: "src/A.java/Child.java", version: 1, content: "class Child {}" },
    ]),
    /Conflicting/,
  );
});

test("accepts only the exact document versions returned for the editor snapshot", () => {
  const documents = [
    { path: "src/A.java", version: 7 },
    { path: "src/B.kt", version: 3 },
  ];
  assert.equal(overlayVersionsMatch(documents, { "src/A.java": 7, "src/B.kt": 3 }), true);
  assert.equal(overlayVersionsMatch(documents, { "src/A.java": 6, "src/B.kt": 3 }), false);
  assert.equal(overlayVersionsMatch(documents, { "src/A.java": 7 }), false);
  assert.equal(overlayVersionsMatch([], {}), true);
  assert.equal(overlayVersionsMatch([], []), false);
});

test("coalesces edits, aborts obsolete work, and publishes only the newest result", async () => {
  const runs = [];
  const states = [];
  const results = [];
  let remoteCancels = 0;
  let releaseFirst;
  const scheduler = new LiveScheduler({
    delay: 100,
    run: (snapshot, signal) => {
      runs.push({ snapshot, signal });
      if (snapshot.version === 1) {
        return new Promise((resolve) => {
          releaseFirst = resolve;
        });
      }
      return Promise.resolve({ status: "ok", version: snapshot.version });
    },
    cancelRemote: () => {
      remoteCancels += 1;
    },
    onState: (state) => states.push(state.state),
    onResult: (result) => results.push(result),
  });

  scheduler.schedule({ version: 1 });
  const first = scheduler.flush();
  await new Promise((resolve) => setImmediate(resolve));
  scheduler.schedule({ version: 2 });
  assert.equal(runs[0].signal.aborted, true);
  releaseFirst({ status: "ok", version: 1 });
  await first;
  await scheduler.flush();

  assert.equal(remoteCancels, 1);
  assert.deepEqual(results, [{ status: "ok", version: 2 }]);
  assert.deepEqual(runs.map((item) => item.snapshot.version), [1, 2]);
  assert.ok(states.includes("queued"));
  assert.equal(states.at(-1), "passed");
  scheduler.dispose();
});

test("only an ok envelope becomes a live pass", async () => {
  const states = [];
  const scheduler = new LiveScheduler({
    delay: 100,
    run: async () => ({ status: "partial", diagnostics: [] }),
    onState: (state) => states.push(state.state),
  });
  scheduler.schedule({ version: 1 });
  await scheduler.flush();
  assert.equal(states.at(-1), "error");
  scheduler.dispose();
});

test("maps compiler failures, analyses, safe values, and coverage to editor presentation", () => {
  const presentation = livePresentation(
    {
      status: "failure",
      revision: "rev-1",
      runId: "run-1",
      data: {
        selectedTests: ["demo.AppTest#fails"],
        tests: ["demo.AppTest#fails"],
        testStatuses: { "demo.AppTest#fails": "failed" },
        overlayVersions: { "src/main/java/demo/App.java": 9 },
        valueHints: [
          {
            testId: "demo.AppTest#fails",
            eventId: "evt-1",
            path: "src/main/java/demo/App.java",
            line: 6,
            label: "middle",
            value: 1,
          },
          {
            testId: "demo.AppTest#fails",
            eventId: "evt-2",
            path: "src/main/java/demo/App.java",
            line: 6,
            label: "middle",
            value: 2,
          },
        ],
        problems: [{
          path: "src/main/java/demo/App.java",
          line: 3,
          column: 5,
          severity: "error",
          message: "cannot find symbol",
        }],
      },
    },
    [{
      data: {
        failure: { id: "failure-1", testId: "demo.AppTest#fails", message: "expected 1" },
        analysis: {
          summary: "Assertion failed: expected 1, observed 2.",
          focus: { path: "src/main/java/demo/App.java", line: 8, column: 1 },
          evidence: [{
            label: "Captured `actual`",
            value: "<redacted>",
            location: { path: "src/main/java/demo/App.java", line: 7, column: 1 },
          }],
        },
      },
    }],
    [{ data: { coverage: [{ path: "src/main/java/demo/App.java", line: 7, testId: "demo.AppTest#fails" }] } }],
  );

  assert.equal(presentation.diagnostics.length, 2);
  assert.equal(presentation.diagnostics[1].code, "failure-1");
  assert.ok(presentation.inlineValues.some((item) => /<redacted>/.test(item.label)));
  assert.ok(presentation.inlineValues.some((item) => item.label === "middle: 1 → 2"));
  assert.equal(presentation.coveredLines[0].line, 7);
  assert.equal(presentation.overlayVersions["src/main/java/demo/App.java"], 9);
  assert.deepEqual(presentation.executedTests, ["demo.AppTest#fails"]);
  assert.equal(presentation.testStatuses["demo.AppTest#fails"], "failed");
});

test("rejects non-canonical result locations before an editor can open them", () => {
  const presentation = livePresentation({
    status: "failure",
    data: {
      problems: [{
        path: "../outside.java",
        line: 1,
        severity: "error",
        message: "untrusted location",
      }],
      valueHints: [{
        testId: "demo.Test#works",
        eventId: "evt-bad",
        path: "../outside.java",
        line: 1,
        label: "secret",
        value: "must not render",
      }],
    },
  }, [], [{
    data: { coverage: [{ path: "src/../outside.java", line: 1, testId: "demo.Test#works" }] },
  }]);

  assert.deepEqual(presentation.diagnostics, []);
  assert.deepEqual(presentation.inlineValues, []);
  assert.deepEqual(presentation.coveredLines, []);
});
