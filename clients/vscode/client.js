"use strict";

const { spawn } = require("node:child_process");

const DEFAULT_MAX_BYTES = 65_536;
const DEFAULT_MAX_STDERR_BYTES = 16_384;
const STATUSES = new Set(["ok", "failure", "partial", "stale", "unsupported", "error"]);
const COMPLETENESS = new Set(["complete", "partial", "unsupported"]);
const SEVERITIES = new Set(["info", "warning", "error"]);
const ENVELOPE_KEYS = [
  "schemaVersion",
  "workspaceId",
  "revision",
  "sessionId",
  "runId",
  "status",
  "data",
  "diagnostics",
  "capabilities",
  "nextActions",
  "page",
];

function invocation(workspace, commandArguments, options = {}) {
  if (typeof workspace !== "string" || workspace.length === 0) {
    throw new TypeError("workspace must be a non-empty string");
  }
  if (!Array.isArray(commandArguments) || !commandArguments.every((value) => typeof value === "string")) {
    throw new TypeError("command arguments must be a string array");
  }
  const environment = options.environment || process.env;
  const maxBytes = boundedBytes(options.maxBytes, DEFAULT_MAX_BYTES);
  const configuredBinary = typeof options.binaryPath === "string" ? options.binaryPath.trim() : "";
  return {
    command: configuredBinary || environment.WALARU_BINARY || "walaru",
    args: [
      "--workspace",
      workspace,
      "--format",
      "json",
      "--max-bytes",
      String(maxBytes),
      ...commandArguments,
    ],
    options: { shell: false, windowsHide: true },
    maxBytes,
  };
}

function decodeEnvelope(chunks, maxBytes = DEFAULT_MAX_BYTES) {
  const payload = Buffer.concat(chunks);
  if (payload.length > maxBytes) {
    throw new Error(`Walaru response exceeded ${maxBytes} bytes`);
  }
  let envelope;
  try {
    envelope = JSON.parse(payload.toString("utf8"));
  } catch (error) {
    throw new Error(`Walaru returned invalid JSON: ${error.message}`);
  }
  validateEnvelope(envelope);
  return envelope;
}

function validateEnvelope(envelope) {
  if (!envelope || typeof envelope !== "object" || Array.isArray(envelope)) {
    throw new Error("Walaru returned an unsupported envelope");
  }
  if (ENVELOPE_KEYS.some((key) => !Object.hasOwn(envelope, key))) {
    throw new Error("Walaru returned an incomplete v1 envelope");
  }
  const capabilities = envelope.capabilities;
  if (
    !hasExactKeys(envelope, ENVELOPE_KEYS) ||
    envelope.schemaVersion !== "1" ||
    !/^ws-[0-9a-f]{16}$/.test(envelope.workspaceId) ||
    !/^rev-[0-9a-f]{64}$/.test(envelope.revision) ||
    typeof envelope.sessionId !== "string" ||
    !(envelope.runId === null || typeof envelope.runId === "string") ||
    !STATUSES.has(envelope.status) ||
    !validDiagnostics(envelope.diagnostics) ||
    !isRecord(capabilities) ||
    !hasExactKeys(capabilities, ["backend", "completeness", "supported", "unavailable"]) ||
    typeof capabilities.backend !== "string" ||
    !COMPLETENESS.has(capabilities.completeness) ||
    !uniqueStrings(capabilities.supported) ||
    !stringMap(capabilities.unavailable) ||
    !validNextActions(envelope.nextActions) ||
    !validPage(envelope.page)
  ) {
    throw new Error("Walaru returned an unsupported envelope");
  }
  return envelope;
}

function validDiagnostics(value) {
  return Array.isArray(value) && value.every((item) => (
    isRecord(item) &&
    hasExactKeys(item, ["code", "severity", "message", "details"]) &&
    typeof item.code === "string" &&
    SEVERITIES.has(item.severity) &&
    typeof item.message === "string" &&
    stringMap(item.details)
  ));
}

function validNextActions(value) {
  return Array.isArray(value) && value.every((item) => (
    isRecord(item) &&
    hasExactKeys(item, ["title", "argv"]) &&
    typeof item.title === "string" &&
    Array.isArray(item.argv) &&
    item.argv.length > 0 &&
    item.argv.every((argument) => typeof argument === "string")
  ));
}

function validPage(value) {
  if (value === null) return true;
  return (
    isRecord(value) &&
    hasExactKeys(value, ["cursor", "nextCursor", "limit", "returned"]) &&
    (value.cursor === null || typeof value.cursor === "string") &&
    (value.nextCursor === null || typeof value.nextCursor === "string") &&
    Number.isSafeInteger(value.limit) &&
    value.limit >= 1 &&
    Number.isSafeInteger(value.returned) &&
    value.returned >= 0
  );
}

function uniqueStrings(value) {
  return (
    Array.isArray(value) &&
    value.every((item) => typeof item === "string") &&
    new Set(value).size === value.length
  );
}

function stringMap(value) {
  return isRecord(value) && Object.values(value).every((item) => typeof item === "string");
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasExactKeys(value, expected) {
  const keys = Object.keys(value);
  return keys.length === expected.length && expected.every((key) => Object.hasOwn(value, key));
}

function query(workspace, commandArguments, options = {}) {
  const environment = options.environment || process.env;
  const call = invocation(workspace, commandArguments, options);
  const maxStderrBytes = boundedBytes(options.maxStderrBytes, DEFAULT_MAX_STDERR_BYTES);
  const spawnProcess = options.spawn || spawn;
  return new Promise((resolve, reject) => {
    const child = spawnProcess(call.command, call.args, {
      ...call.options,
      cwd: workspace,
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    let outputBytes = 0;
    let stderrBytes = 0;
    let exceeded;
    let settled = false;
    child.stdout.on("data", (chunk) => {
      outputBytes += chunk.length;
      if (outputBytes > call.maxBytes) {
        exceeded = `Walaru response exceeded ${call.maxBytes} bytes`;
        child.kill();
        return;
      }
      stdout.push(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes > maxStderrBytes) {
        exceeded = `Walaru stderr exceeded ${maxStderrBytes} bytes`;
        child.kill();
        return;
      }
      stderr.push(chunk);
    });
    child.on("error", (error) => {
      if (!settled) {
        settled = true;
        reject(error);
      }
    });
    child.on("close", (exitCode) => {
      if (settled) return;
      settled = true;
      try {
        if (exceeded) throw new Error(exceeded);
        resolve({
          exitCode: exitCode ?? 3,
          envelope: decodeEnvelope(stdout, call.maxBytes),
          stderr: Buffer.concat(stderr).toString("utf8"),
        });
      } catch (error) {
        reject(error);
      }
    });
  });
}

function boundedBytes(value, fallback) {
  return Number.isSafeInteger(value) && value >= 4_096 && value <= 1_048_576 ? value : fallback;
}

module.exports = {
  DEFAULT_MAX_BYTES,
  DEFAULT_MAX_STDERR_BYTES,
  decodeEnvelope,
  invocation,
  query,
  validateEnvelope,
};
