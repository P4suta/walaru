"use strict";

const path = require("node:path");

const MAX_OVERLAY_DOCUMENTS = 256;
const MAX_OVERLAY_DOCUMENT_BYTES = 1024 * 1024;
const MAX_OVERLAY_BYTES = 4 * 1024 * 1024;
const GENERATED_PATH_SEGMENTS = new Set([
  ".git",
  ".gradle",
  ".idea",
  ".kotlin",
  "node_modules",
]);
const GENERATED_OUTPUT_SEGMENTS = new Set(["build", "dist", "out", "target"]);

class LiveScheduler {
  constructor(options) {
    this.delay = boundedDelay(options.delay);
    this.run = options.run;
    this.cancelRemote = options.cancelRemote || (() => Promise.resolve());
    this.onState = options.onState || (() => {});
    this.onResult = options.onResult || (() => {});
    this.onError = options.onError || (() => {});
    this.generation = 0;
    this.timer = undefined;
    this.active = undefined;
    this.pending = undefined;
    this.paused = false;
    this.disposed = false;
  }

  schedule(snapshot, reason = "edit") {
    if (this.disposed) return;
    this.generation += 1;
    const generation = this.generation;
    this.pending = { generation, snapshot, reason };
    clearTimeout(this.timer);
    if (this.active) {
      this.active.controller.abort();
      Promise.resolve(this.cancelRemote()).catch(() => {});
    }
    if (this.paused) {
      this.onState({ state: "paused", generation, reason });
      return;
    }
    this.onState({ state: "queued", generation, reason });
    this.timer = setTimeout(() => this.start(generation), this.delay);
  }

  async flush() {
    if (!this.pending || this.paused || this.disposed) return;
    clearTimeout(this.timer);
    await this.start(this.pending.generation);
  }

  pause() {
    this.paused = true;
    clearTimeout(this.timer);
    if (this.active) {
      this.active.controller.abort();
      Promise.resolve(this.cancelRemote()).catch(() => {});
    }
    this.onState({ state: "paused", generation: this.generation, reason: "manual" });
  }

  resume() {
    if (!this.paused) return;
    this.paused = false;
    if (this.pending) this.schedule(this.pending.snapshot, "resume");
    else this.onState({ state: "idle", generation: this.generation, reason: "resume" });
  }

  setDelay(delay) {
    this.delay = boundedDelay(delay);
  }

  hasActiveWork() {
    return Boolean(this.active);
  }

  dispose() {
    this.disposed = true;
    clearTimeout(this.timer);
    this.active?.controller.abort();
  }

  async start(generation) {
    if (
      this.disposed ||
      this.paused ||
      !this.pending ||
      generation !== this.generation ||
      generation !== this.pending.generation
    ) {
      return;
    }
    const current = this.pending;
    const controller = new AbortController();
    this.active = { generation, controller };
    const started = Date.now();
    this.onState({ state: "running", generation, reason: current.reason });
    try {
      const result = await this.run(current.snapshot, controller.signal, current.reason);
      if (generation !== this.generation || controller.signal.aborted || this.disposed) return;
      this.pending = undefined;
      this.onResult(result, { generation, elapsedMs: Date.now() - started, reason: current.reason });
      const terminalState = result?.status === "ok"
        ? "passed"
        : result?.status === "failure"
          ? "failed"
          : "error";
      this.onState({
        state: terminalState,
        generation,
        elapsedMs: Date.now() - started,
        reason: current.reason,
        failures: result?.diagnostics?.filter((item) => item.severity === "error").length || 0,
      });
    } catch (error) {
      if (generation !== this.generation || controller.signal.aborted || error?.name === "AbortError") {
        return;
      }
      this.pending = undefined;
      this.onError(error, { generation, elapsedMs: Date.now() - started, reason: current.reason });
      this.onState({
        state: "error",
        generation,
        elapsedMs: Date.now() - started,
        reason: current.reason,
        message: error.message,
      });
    } finally {
      if (this.active?.generation === generation) this.active = undefined;
    }
  }
}

function overlayManifest(sessionId, documents) {
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(sessionId)) {
    throw new Error("Invalid Walaru live session ID");
  }
  if (!Array.isArray(documents) || documents.length > MAX_OVERLAY_DOCUMENTS) {
    throw new Error(`Walaru supports at most ${MAX_OVERLAY_DOCUMENTS} dirty documents`);
  }
  let total = 0;
  const paths = new Set();
  const normalized = documents.map((document) => {
    const relative = normalizeRelativePath(document.path);
    if (paths.has(relative)) throw new Error(`Duplicate Walaru overlay path: ${relative}`);
    if (Array.from(paths).some((existing) => (
      relative.startsWith(`${existing}/`) || existing.startsWith(`${relative}/`)
    ))) {
      throw new Error(`Conflicting Walaru overlay path: ${relative}`);
    }
    paths.add(relative);
    if (!Number.isSafeInteger(document.version)) throw new Error(`Invalid version for ${relative}`);
    if (typeof document.content !== "string") throw new Error(`Invalid contents for ${relative}`);
    const bytes = Buffer.byteLength(document.content, "utf8");
    if (bytes > MAX_OVERLAY_DOCUMENT_BYTES) {
      throw new Error(`Walaru overlay document exceeds ${MAX_OVERLAY_DOCUMENT_BYTES} bytes: ${relative}`);
    }
    total += bytes;
    if (total > MAX_OVERLAY_BYTES) {
      throw new Error(`Walaru overlay payload exceeds ${MAX_OVERLAY_BYTES} bytes`);
    }
    return { path: relative, version: document.version, content: document.content };
  });
  return { schemaVersion: 1, sessionId, documents: normalized };
}

function normalizeRelativePath(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value, "utf8") > 4_096 ||
    value.includes("\\") ||
    /[\u0000-\u001f\u007f]/.test(value)
  ) {
    throw new Error("Walaru overlay paths must be workspace-relative '/' paths");
  }
  const normalized = path.posix.normalize(value);
  if (normalized !== value || normalized === ".." || normalized.startsWith("../") || path.posix.isAbsolute(value)) {
    throw new Error(`Unsafe Walaru overlay path: ${value}`);
  }
  return normalized;
}

function ignoredWorkspacePath(relative) {
  const segments = relative.split("/");
  if (segments.some((segment) => GENERATED_PATH_SEGMENTS.has(segment))) return true;
  const sourceIndex = segments.indexOf("src");
  return segments.some((segment, index) => (
    GENERATED_OUTPUT_SEGMENTS.has(segment) && (sourceIndex < 0 || index < sourceIndex)
  ));
}

function overlayVersionsMatch(documents, versions) {
  if (!versions || typeof versions !== "object" || Array.isArray(versions)) return false;
  const expected = new Map(documents.map((document) => [document.path, document.version]));
  const entries = Object.entries(versions);
  return entries.length === expected.size && entries.every(([path, version]) => (
    Number.isSafeInteger(version) && expected.get(path) === version
  ));
}

function livePresentation(verification, failures = [], coverage = []) {
  const data = verification?.data || {};
  const diagnostics = [];
  const inlineValues = [];
  for (const problem of data.problems || []) {
    if (!validLocation(problem)) continue;
    diagnostics.push({
      path: problem.path,
      line: problem.line,
      column: problem.column || 1,
      severity: problem.severity === "warning" ? "warning" : "error",
      message: problem.message,
      code: "WALARU_COMPILER",
    });
  }
  for (const envelope of failures) {
    const failure = envelope?.data?.failure;
    const analysis = envelope?.data?.analysis;
    if (analysis?.focus && validLocation(analysis.focus)) {
      diagnostics.push({
        path: analysis.focus.path,
        line: analysis.focus.line,
        column: analysis.focus.column || 1,
        severity: "error",
        message: analysis.summary || failure?.message || "Walaru test failed",
        code: failure?.id || "WALARU_TEST_FAILURE",
        testId: failure?.testId,
      });
    }
    for (const evidence of analysis?.evidence || []) {
      if (!evidence.location || !validLocation(evidence.location)) continue;
      inlineValues.push({
        path: evidence.location.path,
        line: evidence.location.line,
        name: evidence.label,
        value: compactValue(evidence.value),
        hover: analysis.summary || failure?.message || evidence.label,
      });
    }
  }
  for (const hint of Array.isArray(data.valueHints) ? data.valueHints : []) {
    if (
      !validLocation(hint) ||
      typeof hint.label !== "string" ||
      hint.label.length === 0 ||
      hint.label.length > 256 ||
      typeof hint.eventId !== "string" ||
      typeof hint.testId !== "string"
    ) {
      continue;
    }
    inlineValues.push({
      path: hint.path,
      line: hint.line,
      name: hint.label,
      value: compactValue(hint.value),
      hover: `${hint.testId} · ${hint.eventId}`,
    });
  }
  const coveredLines = [];
  for (const envelope of coverage) {
    for (const item of envelope?.data?.coverage || []) {
      if (validLocation(item)) coveredLines.push({ path: item.path, line: item.line, testId: item.testId });
    }
  }
  return {
    status: verification?.status || "error",
    revision: verification?.revision || "",
    runId: verification?.runId || null,
    overlayVersions: data.overlayVersions || {},
    executedTests: Array.isArray(data.tests) ? data.tests : [],
    testStatuses: isStringMap(data.testStatuses) ? data.testStatuses : {},
    selectedTests: data.selectedTests || [],
    diagnostics: deduplicate(diagnostics, (item) => `${item.path}:${item.line}:${item.column}:${item.message}`),
    inlineValues: coalesceInlineValues(inlineValues),
    coveredLines: deduplicate(coveredLines, (item) => `${item.path}:${item.line}`),
  };
}

function coalesceInlineValues(values) {
  const grouped = new Map();
  for (const item of values) {
    const key = `${item.path}:${item.line}:${item.name}`;
    const current = grouped.get(key) || { ...item, values: [], total: 0 };
    if (!current.values.includes(item.value)) {
      current.total += 1;
      if (current.values.length < 3) current.values.push(item.value);
    }
    if (!current.hover.includes(item.hover)) current.hover = `${current.hover}\n${item.hover}`;
    grouped.set(key, current);
  }
  return Array.from(grouped.values()).map((item) => ({
    path: item.path,
    line: item.line,
    label: `${item.name}: ${item.values.join(" → ")}${item.total > item.values.length ? ` … (+${item.total - item.values.length})` : ""}`,
    hover: item.hover,
  }));
}

function isStringMap(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    && Object.values(value).every((item) => typeof item === "string");
}

function validLocation(value) {
  if (
    !value ||
    typeof value.path !== "string" ||
    !Number.isSafeInteger(value.line) ||
    value.line <= 0 ||
    (value.column != null && (!Number.isSafeInteger(value.column) || value.column <= 0))
  ) {
    return false;
  }
  try {
    return normalizeRelativePath(value.path) === value.path;
  } catch {
    return false;
  }
}

function compactValue(value) {
  if (typeof value === "string") return value.length > 80 ? `${value.slice(0, 77)}…` : value;
  const encoded = JSON.stringify(value);
  if (!encoded) return String(value);
  return encoded.length > 80 ? `${encoded.slice(0, 77)}…` : encoded;
}

function deduplicate(values, key) {
  const seen = new Set();
  return values.filter((value) => {
    const current = key(value);
    if (seen.has(current)) return false;
    seen.add(current);
    return true;
  });
}

function boundedDelay(value) {
  const number = Number(value);
  return Number.isFinite(number) ? Math.max(100, Math.min(5_000, Math.round(number))) : 500;
}

module.exports = {
  LiveScheduler,
  ignoredWorkspacePath,
  livePresentation,
  normalizeRelativePath,
  overlayManifest,
  overlayVersionsMatch,
};
