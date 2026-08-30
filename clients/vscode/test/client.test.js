"use strict";

const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const test = require("node:test");
const { decodeEnvelope, invocation, query, validateEnvelope } = require("../client");

function envelope(overrides = {}) {
  return {
    schemaVersion: "1",
    workspaceId: "ws-0123456789abcdef",
    revision: `rev-${"a".repeat(64)}`,
    sessionId: "session-1",
    runId: null,
    status: "ok",
    data: {},
    diagnostics: [],
    capabilities: {
      backend: "none",
      completeness: "unsupported",
      supported: [],
      unavailable: {},
    },
    nextActions: [],
    page: null,
    ...overrides,
  };
}

test("builds a direct resource-configured argv invocation and preserves spaces", () => {
  const call = invocation("/work/project with spaces", ["impact", "a; touch /tmp/nope"], {
    environment: {},
    binaryPath: "/opt/walaru/bin/walaru",
  });
  assert.equal(call.command, "/opt/walaru/bin/walaru");
  assert.equal(call.options.shell, false);
  assert.deepEqual(call.args.slice(0, 7), [
    "--workspace",
    "/work/project with spaces",
    "--format",
    "json",
    "--max-bytes",
    "65536",
    "impact",
  ]);
  assert.equal(call.args.at(-1), "a; touch /tmp/nope");
});

test("validates the complete v1 structured envelope", () => {
  assert.equal(decodeEnvelope([Buffer.from(JSON.stringify(envelope()))]).status, "ok");
  assert.throws(() => validateEnvelope({ schemaVersion: "1" }), /incomplete/);
  assert.throws(() => validateEnvelope(envelope({ schemaVersion: "2" })), /unsupported/);
  assert.throws(() => validateEnvelope(envelope({ status: "mystery" })), /unsupported/);
  assert.throws(() => validateEnvelope(envelope({ workspaceId: "workspace" })), /unsupported/);
  assert.throws(() => validateEnvelope(envelope({ runId: 7 })), /unsupported/);
  assert.throws(() => validateEnvelope(envelope({ data: [] })), /unsupported/);
  assert.throws(
    () => validateEnvelope(envelope({ diagnostics: [{ code: "x", severity: "fatal", message: "x", details: {} }] })),
    /unsupported/,
  );
  assert.throws(
    () => validateEnvelope(envelope({ capabilities: { backend: "none", completeness: "maybe", supported: [], unavailable: {} } })),
    /unsupported/,
  );
  assert.throws(
    () => validateEnvelope(envelope({ nextActions: [{ title: "retry", argv: [] }] })),
    /unsupported/,
  );
  assert.throws(
    () => validateEnvelope(envelope({ page: { cursor: null, nextCursor: null, limit: 0, returned: 0 } })),
    /unsupported/,
  );
  assert.throws(() => validateEnvelope({ ...envelope(), extra: true }), /unsupported/);
  assert.throws(() => decodeEnvelope([Buffer.from("not-json")]), /invalid JSON/);
});

test("rejects structured responses beyond the configured limit", async () => {
  assert.throws(() => decodeEnvelope([Buffer.alloc(65_537)]), /exceeded/);
  const fakeSpawn = () => {
    const child = new EventEmitter();
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.kill = () => {};
    process.nextTick(() => {
      child.stdout.emit("data", Buffer.alloc(5_000));
      child.emit("close", 4);
    });
    return child;
  };
  await assert.rejects(
    query("/work/project", ["tests"], { spawn: fakeSpawn, maxBytes: 4_096 }),
    /exceeded 4096 bytes/,
  );
});

test("cancels a shell-free child through AbortSignal and rejects only after close", async () => {
  let killed = false;
  const fakeSpawn = () => {
    const child = new EventEmitter();
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.kill = () => {
      killed = true;
      process.nextTick(() => child.emit("close", null));
    };
    return child;
  };
  const controller = new AbortController();
  const result = query("/work/project", ["verify", "--supersede"], {
    spawn: fakeSpawn,
    signal: controller.signal,
  });
  controller.abort();
  await assert.rejects(result, (error) => error.name === "AbortError");
  assert.equal(killed, true);
});
