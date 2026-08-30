"use strict";

const FAILURE_RANK = new Map([
  ["failed", 0],
  ["passed", 1],
]);

function workspaceModel(folder, statusEnvelope, testsEnvelope) {
  if (!folder || !folder.uri || typeof folder.uri.fsPath !== "string") {
    throw new TypeError("workspace folder is required");
  }
  const tests = (testsEnvelope?.data?.tests || []).map((test) => ({
    id: String(test.id || ""),
    displayName: String(test.displayName || test.id || ""),
    module: String(test.module || ":"),
    status: test.lastStatus == null ? "unknown" : String(test.lastStatus),
    lastFailureId: test.lastFailureId == null ? null : String(test.lastFailureId),
  }));
  tests.sort(compareTests);
  return {
    folder,
    workspaceId: statusEnvelope?.workspaceId || testsEnvelope?.workspaceId || "",
    revision: statusEnvelope?.revision || testsEnvelope?.revision || "",
    daemonRunning: statusEnvelope?.data?.running === true,
    tests,
    error: null,
  };
}

function compareTests(left, right) {
  const rank = (status) => FAILURE_RANK.get(status) ?? 2;
  return (
    rank(left.status) - rank(right.status) ||
    left.module.localeCompare(right.module) ||
    left.id.localeCompare(right.id)
  );
}

function applyLiveStatuses(current, testStatuses, revision) {
  if (!current || !testStatuses || typeof testStatuses !== "object" || Array.isArray(testStatuses)) {
    return current;
  }
  const tests = current.tests.map((test) => ({
    ...test,
    status: typeof testStatuses[test.id] === "string" ? testStatuses[test.id] : test.status,
  }));
  tests.sort(compareTests);
  return {
    ...current,
    revision: revision || current.revision,
    tests,
    error: null,
  };
}

function statusSummary(models, trusted = true, liveStates = []) {
  if (!trusted) {
    return {
      text: "$(lock) Walaru: workspace untrusted",
      tooltip: "Trust the workspace to run Walaru",
      failures: 0,
    };
  }
  const list = Array.from(models || []);
  const live = Array.from(liveStates || []);
  const tests = list.reduce((count, current) => count + current.tests.length, 0);
  const failures = list.reduce(
    (count, current) => count + current.tests.filter((test) => test.status === "failed").length,
    0,
  );
  const errors = list.filter((current) => current.error).length;
  const liveErrors = live.filter((current) => current.state === "error").length;
  const running = live.filter((current) => current.state === "running").length;
  const queued = live.filter((current) => current.state === "queued").length;
  const dirty = live.filter((current) => current.state === "dirty").length;
  const paused = live.filter((current) => current.state === "paused").length;
  const liveFailures = live.reduce(
    (count, current) => count + (current.state === "failed" ? Math.max(1, current.failures || 0) : 0),
    0,
  );
  if (running > 0) {
    return {
      text: `$(sync~spin) Walaru: checking ${running} workspace${running === 1 ? "" : "s"}`,
      tooltip: "Unsaved buffers are being verified; newer edits replace this run",
      failures: Math.max(failures, liveFailures),
    };
  }
  if (queued > 0) {
    return {
      text: "$(watch) Walaru: edit queued",
      tooltip: "Live verification will start after the debounce window",
      failures: Math.max(failures, liveFailures),
    };
  }
  if (dirty > 0) {
    return {
      text: "$(circle-outline) Walaru: save to verify",
      tooltip: "The editor changed; on-save live verification is waiting for a save",
      failures: Math.max(failures, liveFailures),
    };
  }
  if (errors > 0 || liveErrors > 0) {
    return {
      text: `$(error) Walaru: ${errors + liveErrors} workspace error${errors + liveErrors === 1 ? "" : "s"}`,
      tooltip: "Open Walaru output for details",
      failures: Math.max(failures, liveFailures),
    };
  }
  if (paused > 0 && paused === live.length) {
    return {
      text: "$(debug-pause) Walaru: live paused",
      tooltip: "Run Walaru: Resume Live Verification to continue",
      failures: Math.max(failures, liveFailures),
    };
  }
  const currentFailures = Math.max(failures, liveFailures);
  const icon = currentFailures > 0 ? "$(testing-failed-icon)" : "$(testing-passed-icon)";
  return {
    text: `${icon} Walaru: ${currentFailures} failed / ${tests}`,
    tooltip: `${list.length} workspace${list.length === 1 ? "" : "s"}; ${tests} tests`,
    failures: currentFailures,
  };
}

function commandArguments(command, test) {
  switch (command) {
    case "failure":
      if (!test?.lastFailureId) throw new Error("The selected test has no recorded failure");
      return ["failure", test.lastFailureId];
    case "trace":
      if (!test?.id) throw new Error("A test is required");
      return ["trace", test.id];
    case "record":
      if (!test?.id) throw new Error("A test is required");
      return ["record", test.id];
    case "verify":
      return ["verify"];
    case "fullVerify":
      return ["verify", "--full"];
    case "status":
    case "doctor":
    case "tests":
      return [command];
    default:
      throw new Error(`Unsupported Walaru command: ${command}`);
  }
}

function executionAllowed(workspaceTrusted) {
  return workspaceTrusted === true;
}

function retainWorkspaceModels(models, folders) {
  const active = new Set((folders || []).map((folder) => folder.uri.toString()));
  for (const key of models.keys()) {
    if (!active.has(key)) models.delete(key);
  }
  return models;
}

function formatEnvelope(command, envelope) {
  const lines = [`Walaru ${command} · ${envelope.status}`, `Revision: ${envelope.revision}`];
  const data = envelope.data || {};
  if (command === "tests") {
    const tests = data.tests || [];
    lines.push(`Tests: ${tests.length}`, "", "STATUS   MODULE           TEST");
    for (const test of tests) {
      lines.push(`${pad(test.lastStatus || "unknown", 8)} ${pad(test.module || ":", 16)} ${test.id}`);
    }
  } else if (command === "failure") {
    const failure = data.failure;
    if (!failure) lines.push("No failure found.");
    else {
      lines.push(
        `Failure: ${failure.id}`,
        `Test: ${failure.testId}`,
        `Type: ${failure.exceptionType}`,
        `Message: ${failure.message}`,
      );
      const analysis = data.analysis;
      if (analysis) {
        const focus = analysis.focus
          ? `${analysis.focus.path}:${analysis.focus.line}`
          : "-";
        lines.push(
          "",
          "Why this probably failed",
          `  ${analysis.summary}`,
          `  Evidence: ${analysis.likelyCause}`,
          `  Focus: ${focus}`,
        );
        if (analysis.evidence?.length) {
          lines.push("", "Relevant state:");
          for (const item of analysis.evidence.slice(0, 7)) {
            lines.push(`  ${item.label} = ${compact(item.value)}`);
          }
        }
        if (analysis.suggestions?.length) {
          lines.push("", "Try next:");
          for (const suggestion of analysis.suggestions.slice(0, 4)) {
            lines.push(`  - ${suggestion}`);
          }
        }
      }
      if (failure.frames?.length) {
        lines.push("", "Top stack frames:", ...failure.frames.slice(0, 8).map((frame) => `  ${frame}`));
        if (failure.frames.length > 8) lines.push(`  … ${failure.frames.length - 8} more`);
      }
    }
  } else if (command === "trace") {
    const events = data.events || [];
    lines.push(`Events: ${events.length}`, "", "SEQ      KIND         SOURCE                         THREAD");
    for (const item of events) {
      const location = item.location ? `${item.location.path}:${item.location.line}` : "-";
      lines.push(`${pad(item.sequence, 8)} ${pad(item.kind, 12)} ${pad(location, 30)} ${item.threadId}`);
    }
  } else if (command === "doctor") {
    lines.push(
      `Ready: ${data.ready === true ? "yes" : "no"}`,
      `Platform: ${data.platform?.os || "?"}/${data.platform?.arch || "?"}`,
      `JDK: ${data.java?.major || "missing"}`,
      `Build tool: ${data.buildTool?.kind || "missing"}`,
      `Runtime artifacts: ${data.runtimeArtifacts?.present === true ? "present" : "missing"}`,
    );
  } else if (command === "verify") {
    lines.push(
      `Verification: ${data.status || envelope.status}`,
      `Run: ${envelope.runId || "-"}`,
      `Tests: ${(data.tests || []).length}`,
      `Events: ${data.events ?? "-"}`,
    );
  } else if (command === "record") {
    lines.push(
      `Recording: ${data.recordingId || "-"}`,
      `Test: ${data.testId || "-"}`,
      `Events: ${data.events ?? "-"}`,
      `Completeness: ${envelope.capabilities?.completeness || "unknown"}`,
    );
  } else if (command === "status") {
    lines.push(
      `Daemon: ${data.running === true ? "running" : "stopped"}`,
      `Process: ${data.pid || "-"}`,
      `Workspace: ${envelope.workspaceId}`,
    );
  } else {
    lines.push(...summaryPairs(data));
  }
  if (envelope.diagnostics?.length) {
    lines.push("", "Diagnostics:");
    for (const diagnostic of envelope.diagnostics) {
      lines.push(`  [${diagnostic.severity}] ${diagnostic.code}: ${diagnostic.message}`);
    }
  }
  if (envelope.nextActions?.length) {
    lines.push("", "Next actions:");
    for (const action of envelope.nextActions) {
      lines.push(`  ${action.title}`, `    argv: ${action.argv.join(" | ")}`);
    }
  }
  return lines.filter((line) => line !== undefined).join("\n");
}

function summaryPairs(value) {
  if (!value || typeof value !== "object") return [String(value ?? "-")];
  return Object.entries(value)
    .slice(0, 20)
    .map(([key, item]) => `${key}: ${compact(item)}`);
}

function compact(value) {
  if (Array.isArray(value)) return `${value.length} items`;
  if (value && typeof value === "object") {
    return Object.entries(value)
      .map(([key, item]) => `${key}=${compact(item)}`)
      .join(", ");
  }
  return String(value ?? "-");
}

function pad(value, width) {
  const text = String(value ?? "-");
  return text.length >= width ? `${text.slice(0, width - 1)}…` : text.padEnd(width);
}

module.exports = {
  applyLiveStatuses,
  commandArguments,
  compareTests,
  executionAllowed,
  formatEnvelope,
  retainWorkspaceModels,
  statusSummary,
  workspaceModel,
};
