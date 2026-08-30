"use strict";

const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const vscode = require("vscode");
const client = require("./client");
const model = require("./model");
const {
  LiveScheduler,
  ignoredWorkspacePath,
  livePresentation,
  normalizeRelativePath,
  overlayManifest,
  overlayVersionsMatch,
} = require("./live");

const BUILD_INPUT_NAMES = new Set([
  "pom.xml",
  "gradle.properties",
  "libs.versions.toml",
  "maven.config",
  "jvm.config",
  "extensions.xml",
  "gradle-wrapper.properties",
]);

class WalaruTreeProvider {
  constructor(output, statusBar) {
    this.output = output;
    this.statusBar = statusBar;
    this.models = new Map();
    this.liveStates = new Map();
    this.refreshing = new Set();
    this.timers = new Map();
    this.visible = false;
    this.emitter = new vscode.EventEmitter();
    this.onDidChangeTreeData = this.emitter.event;
  }

  dispose() {
    this.stopTimers();
    this.emitter.dispose();
  }

  getTreeItem(element) {
    if (element.kind === "workspace") {
      const item = new vscode.TreeItem(
        element.folder.name,
        vscode.TreeItemCollapsibleState.Expanded,
      );
      const live = this.liveStates.get(element.folder.uri.toString());
      item.description = live?.state === "running"
        ? "checking…"
        : live?.state === "queued"
          ? "edit queued"
          : live?.state === "failed"
            ? "live failure"
            : live?.state === "error"
              ? "live error"
        : element.model.error
          ? "unavailable"
          : `${element.model.tests.length} tests`;
      item.contextValue = "walaruWorkspace";
      item.iconPath = new vscode.ThemeIcon(
        live?.state === "running"
          ? "sync~spin"
          : ["failed", "error"].includes(live?.state) || element.model.error
            ? "error"
            : "folder-library",
      );
      return item;
    }
    if (element.kind === "message") {
      const item = new vscode.TreeItem(element.label, vscode.TreeItemCollapsibleState.None);
      item.iconPath = new vscode.ThemeIcon("lock");
      return item;
    }
    const item = new vscode.TreeItem(element.test.displayName, vscode.TreeItemCollapsibleState.None);
    item.description = `${element.test.module} · ${element.test.status}`;
    item.tooltip = `${element.test.id}\n${element.test.lastFailureId || "No recorded failure"}`;
    item.contextValue = element.test.lastFailureId ? "walaruFailedTest" : "walaruTest";
    item.iconPath = new vscode.ThemeIcon(
      element.test.status === "failed"
        ? "testing-failed-icon"
        : element.test.status === "passed"
          ? "testing-passed-icon"
          : "testing-unset-icon",
    );
    item.command = {
      command: "walaru.trace",
      title: "Open Walaru Trace",
      arguments: [element],
    };
    return item;
  }

  getChildren(element) {
    if (!vscode.workspace.isTrusted) {
      return element ? [] : [{ kind: "message", label: "Trust this workspace to run Walaru" }];
    }
    if (!element) {
      return (vscode.workspace.workspaceFolders || []).map((folder) => ({
        kind: "workspace",
        folder,
        model: this.models.get(folder.uri.toString()) || emptyModel(folder),
      }));
    }
    if (element.kind !== "workspace") return [];
    return element.model.tests.map((test) => ({ kind: "test", folder: element.folder, test }));
  }

  allTests() {
    const items = [];
    for (const current of this.models.values()) {
      for (const test of current.tests) items.push({ kind: "test", folder: current.folder, test });
    }
    return items;
  }

  setVisible(visible) {
    this.visible = visible;
    if (visible) {
      this.refreshAll();
      this.startTimers();
    } else {
      this.stopTimers();
    }
  }

  setLiveState(folder, state) {
    this.liveStates.set(folder.uri.toString(), state);
    this.updateStatus();
    this.emitter.fire();
  }

  applyLiveResult(folder, result) {
    const key = folder.uri.toString();
    const current = this.models.get(key);
    if (!current) return;
    this.models.set(
      key,
      model.applyLiveStatuses(current, result.testStatuses, result.revision),
    );
    this.onModelChange?.();
    this.updateStatus();
    this.emitter.fire();
  }

  removeWorkspace(folder) {
    const key = folder.uri.toString();
    this.models.delete(key);
    this.liveStates.delete(key);
  }

  startTimers() {
    this.stopTimers();
    if (!this.visible || !vscode.workspace.isTrusted) return;
    for (const folder of vscode.workspace.workspaceFolders || []) {
      const seconds = configuration(folder).get("refreshIntervalSeconds", 10);
      const interval = Math.max(2, Math.min(300, Number(seconds) || 10)) * 1_000;
      this.timers.set(folder.uri.toString(), setInterval(() => this.refresh(folder), interval));
    }
  }

  stopTimers() {
    for (const timer of this.timers.values()) clearInterval(timer);
    this.timers.clear();
  }

  async refreshAll() {
    model.retainWorkspaceModels(this.models, vscode.workspace.workspaceFolders);
    if (!vscode.workspace.isTrusted) {
      this.models.clear();
      this.updateStatus();
      this.emitter.fire();
      return;
    }
    await Promise.all((vscode.workspace.workspaceFolders || []).map((folder) => this.refresh(folder)));
  }

  syncWorkspaceFolders() {
    model.retainWorkspaceModels(this.models, vscode.workspace.workspaceFolders);
    this.onModelChange?.();
    this.updateStatus();
    this.emitter.fire();
  }

  async refresh(folder) {
    if (!vscode.workspace.isTrusted) return;
    const key = folder.uri.toString();
    if (this.refreshing.has(key)) return;
    this.refreshing.add(key);
    try {
      const [status, tests] = await Promise.all([
        runQuery(folder, ["status"]),
        runQuery(folder, ["tests"]),
      ]);
      this.models.set(key, model.workspaceModel(folder, status.envelope, tests.envelope));
    } catch (error) {
      const current = this.models.get(key) || emptyModel(folder);
      current.error = error.message;
      this.models.set(key, current);
      this.write(folder, "refresh", error.message);
    } finally {
      this.refreshing.delete(key);
      this.onModelChange?.();
      this.updateStatus();
      this.emitter.fire();
    }
  }

  updateStatus() {
    const summary = model.statusSummary(
      this.models.values(),
      vscode.workspace.isTrusted,
      this.liveStates.values(),
    );
    this.statusBar.text = summary.text;
    this.statusBar.tooltip = summary.tooltip;
    this.statusBar.backgroundColor = summary.failures > 0
      ? new vscode.ThemeColor("statusBarItem.errorBackground")
      : undefined;
    this.statusBar.show();
  }

  write(folder, command, contents) {
    this.output.appendLine(`\n=== ${folder.name} · ${command} ===`);
    this.output.appendLine(contents);
  }
}

class WalaruTestIntegration {
  constructor(provider, live, output) {
    this.provider = provider;
    this.live = live;
    this.output = output;
    this.controller = vscode.tests.createTestController("walaru", "Walaru");
    this.metadata = new Map();
    this.profile = this.controller.createRunProfile(
      "Run with Walaru",
      vscode.TestRunProfileKind.Run,
      (request, token) => this.run(request, token),
      true,
    );
    this.controller.refreshHandler = () => this.provider.refreshAll();
    this.sync();
  }

  dispose() {
    this.profile.dispose();
    this.controller.dispose();
  }

  sync() {
    const roots = [];
    const metadata = new Map();
    for (const folder of vscode.workspace.workspaceFolders || []) {
      const root = this.controller.createTestItem(
        `workspace:${folder.uri.toString()}`,
        folder.name,
        folder.uri,
      );
      root.description = folder.uri.fsPath;
      const current = this.provider.models.get(folder.uri.toString()) || emptyModel(folder);
      const children = current.tests.map((test) => {
        const item = this.controller.createTestItem(
          `test:${folder.uri.toString()}:${test.id}`,
          test.displayName,
          folder.uri,
        );
        item.description = `${test.module} · ${test.status}`;
        item.error = test.lastFailureId ? `Last failure: ${test.lastFailureId}` : undefined;
        metadata.set(item.id, { folder, test, item, root: false });
        return item;
      });
      root.children.replace(children);
      metadata.set(root.id, { folder, item: root, root: true });
      roots.push(root);
    }
    this.metadata = metadata;
    this.controller.items.replace(roots);
  }

  invalidate() {
    this.controller.invalidateTestResults();
  }

  publishLive(folder, result, timing) {
    const key = folder.uri.toString();
    const entries = Array.from(this.metadata.values())
      .filter((entry) => !entry.root && entry.folder.uri.toString() === key);
    const statuses = new Map(Object.entries(result.testStatuses || {}));
    const affected = entries.filter((entry) => statuses.has(entry.test.id));
    const compilerProblems = result.diagnostics.filter((item) => !item.testId);
    const root = Array.from(this.metadata.values())
      .find((entry) => entry.root && entry.folder.uri.toString() === key);
    const items = affected.map((entry) => entry.item);
    if (items.length === 0 && result.status !== "ok" && root) items.push(root.item);
    if (items.length === 0) return;

    const request = new vscode.TestRunRequest(items, undefined, undefined, false, true);
    const run = this.controller.createTestRun(
      request,
      `Walaru Live · ${(timing.elapsedMs / 1000).toFixed(2)}s`,
      false,
    );
    for (const item of items) run.started(item);
    if (affected.length === 0 && root) {
      const messages = compilerProblems.map((item) => presentationMessage(item, folder));
      run.errored(
        root.item,
        messages.length > 0
          ? messages
          : new vscode.TestMessage(`Walaru live verification returned ${result.status}`),
      );
      run.end();
      return;
    }
    for (const entry of affected) {
      const status = statuses.get(entry.test.id);
      if (status === "passed") {
        run.passed(entry.item);
      } else if (status === "skipped") {
        run.skipped(entry.item);
      } else if (status === "failed") {
        const messages = result.diagnostics
          .filter((item) => item.testId === entry.test.id)
          .map((item) => presentationMessage(item, folder));
        run.failed(
          entry.item,
          messages.length > 0 ? messages : new vscode.TestMessage(`${entry.test.id} failed`),
        );
      } else {
        run.errored(entry.item, new vscode.TestMessage(`Walaru returned test status: ${status}`));
      }
    }
    run.end();
  }

  async run(request, token) {
    if (!vscode.workspace.isTrusted) {
      vscode.window.showWarningMessage("Trust this workspace before running Walaru tests.");
      return;
    }
    const run = this.controller.createTestRun(request, "Walaru", true);
    const selected = this.selected(request);
    const groups = new Map();
    for (const entry of selected) {
      const key = entry.folder.uri.toString();
      const group = groups.get(key) || { folder: entry.folder, entries: [], runAll: false };
      group.entries.push(entry);
      group.runAll ||= entry.root;
      groups.set(key, group);
      run.enqueued(entry.item);
    }
    const abort = new AbortController();
    const cancellation = token.onCancellationRequested(() => {
      abort.abort();
      for (const group of groups.values()) runQuery(group.folder, ["cancel"]).catch(() => {});
    });
    try {
      for (const group of groups.values()) {
        if (token.isCancellationRequested) break;
        this.live.suspend(group.folder);
        for (const entry of group.entries) run.started(entry.item);
        const exact = group.entries.filter((entry) => !entry.root);
        const args = group.runAll || exact.length === 0
          ? ["verify", "--full", "--supersede"]
          : ["verify", "--supersede", ...exact.flatMap((entry) => ["--test", entry.test.id])];
        try {
          const result = await this.live.verify(group.folder, args, {
            signal: abort.signal,
            maxBytes: 1024 * 1024,
          });
          run.appendOutput(`${model.formatEnvelope("verify", result.envelope).replaceAll("\n", "\r\n")}\r\n`);
          const failures = await Promise.all(
            (result.envelope.data?.failures || []).slice(0, 20).map(async (id) => (
              await runQuery(group.folder, ["failure", id], { signal: abort.signal })
            ).envelope),
          );
          const statuses = new Map(Object.entries(result.envelope.data?.testStatuses || {}));
          const failureByTest = new Map(
            failures
              .filter((envelope) => envelope.data?.failure?.testId)
              .map((envelope) => [envelope.data.failure.testId, envelope]),
          );
          for (const entry of group.entries) {
            if (entry.root) {
              if (result.envelope.status === "ok") {
                run.passed(entry.item);
              } else if (result.envelope.status === "failure") {
                run.failed(entry.item, problemMessages(result.envelope, group.folder));
              } else {
                run.errored(
                  entry.item,
                  new vscode.TestMessage(`Walaru verification returned ${result.envelope.status}`),
                );
              }
              continue;
            }
            const status = statuses.get(entry.test.id);
            if (status === "failed") {
              run.failed(
                entry.item,
                testMessages(failureByTest.get(entry.test.id), result.envelope, group.folder),
              );
            } else if (status === "passed") {
              run.passed(entry.item);
            } else if (status === "skipped") {
              run.skipped(entry.item);
            } else {
              run.errored(
                entry.item,
                new vscode.TestMessage(`No fresh terminal evidence for ${entry.test.id}`),
              );
            }
          }
          await this.provider.refresh(group.folder);
        } catch (error) {
          for (const entry of group.entries) {
            if (error.name === "AbortError") run.skipped(entry.item);
            else run.errored(entry.item, new vscode.TestMessage(error.message));
          }
        } finally {
          this.live.release(group.folder);
        }
      }
    } finally {
      cancellation.dispose();
      run.end();
    }
  }

  selected(request) {
    const excluded = new Set();
    for (const item of request.exclude || []) collectTestIds(item, excluded);
    const entries = [];
    const add = (item) => {
      const entry = this.metadata.get(item.id);
      if (entry && !excluded.has(item.id)) entries.push(entry);
      if (entry?.root) item.children.forEach(add);
    };
    if (request.include) request.include.forEach(add);
    else this.controller.items.forEach(add);
    const unique = new Map(entries.map((entry) => [entry.item.id, entry]));
    return Array.from(unique.values());
  }
}

function collectTestIds(item, output) {
  output.add(item.id);
  item.children.forEach((child) => collectTestIds(child, output));
}

function problemMessages(envelope, folder) {
  const messages = (envelope.data?.problems || []).slice(0, 20).flatMap((problem) => {
    const location = sourceLocation(folder, problem);
    if (!location) return [];
    const message = new vscode.TestMessage(problem.message);
    message.location = location;
    return [message];
  });
  return messages.length > 0 ? messages : [new vscode.TestMessage("Walaru verification failed")];
}

function testMessages(failureEnvelope, verificationEnvelope, folder) {
  const failure = failureEnvelope?.data?.failure;
  const analysis = failureEnvelope?.data?.analysis;
  if (!failure && !analysis) return problemMessages(verificationEnvelope, folder);
  const message = new vscode.TestMessage(
    analysis?.summary || failure?.message || "Walaru test failed",
  );
  message.location = sourceLocation(folder, analysis?.focus);
  return [message];
}

function presentationMessage(diagnostic, folder) {
  const message = new vscode.TestMessage(diagnostic.message);
  message.location = sourceLocation(folder, diagnostic);
  return message;
}

function sourceLocation(folder, location) {
  if (!location || !Number.isSafeInteger(location.line) || location.line <= 0) return undefined;
  if (
    location.column != null &&
    (!Number.isSafeInteger(location.column) || location.column <= 0)
  ) {
    return undefined;
  }
  let relative;
  try {
    relative = normalizeRelativePath(location.path);
  } catch {
    return undefined;
  }
  return new vscode.Location(
    vscode.Uri.file(path.join(folder.uri.fsPath, ...relative.split("/"))),
    new vscode.Position(location.line - 1, (location.column || 1) - 1),
  );
}

class LiveDecorations {
  constructor(context) {
    this.presentations = new Map();
    this.diagnosticUris = new Map();
    this.diagnostics = vscode.languages.createDiagnosticCollection("walaru");
    this.failures = vscode.window.createTextEditorDecorationType({
      isWholeLine: true,
      gutterIconPath: vscode.Uri.file(context.asAbsolutePath("media/failed.svg")),
      gutterIconSize: "contain",
      overviewRulerColor: new vscode.ThemeColor("editorError.foreground"),
      overviewRulerLane: vscode.OverviewRulerLane.Right,
    });
    this.values = vscode.window.createTextEditorDecorationType({
      rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed,
    });
    this.coverage = vscode.window.createTextEditorDecorationType({
      isWholeLine: true,
      gutterIconPath: vscode.Uri.file(context.asAbsolutePath("media/covered.svg")),
      gutterIconSize: "contain",
    });
  }

  dispose() {
    this.diagnostics.dispose();
    this.failures.dispose();
    this.values.dispose();
    this.coverage.dispose();
  }

  apply(folder, presentation) {
    this.clear(folder);
    const key = folder.uri.toString();
    this.presentations.set(key, { folder, presentation });
    const grouped = new Map();
    for (const item of presentation.diagnostics) {
      const uri = vscode.Uri.file(path.join(folder.uri.fsPath, ...item.path.split("/")));
      const uriKey = uri.toString();
      const list = grouped.get(uriKey) || { uri, diagnostics: [] };
      const line = Math.max(0, item.line - 1);
      const column = Math.max(0, (item.column || 1) - 1);
      const diagnostic = new vscode.Diagnostic(
        new vscode.Range(line, column, line, column + 1),
        item.message,
        item.severity === "warning"
          ? vscode.DiagnosticSeverity.Warning
          : vscode.DiagnosticSeverity.Error,
      );
      diagnostic.source = "Walaru";
      diagnostic.code = item.code;
      list.diagnostics.push(diagnostic);
      grouped.set(uriKey, list);
    }
    const uris = new Set();
    for (const [uriKey, value] of grouped) {
      this.diagnostics.set(value.uri, value.diagnostics);
      uris.add(uriKey);
    }
    this.diagnosticUris.set(key, uris);
    this.refreshVisibleEditors();
  }

  clear(folder) {
    const key = folder.uri.toString();
    for (const uri of this.diagnosticUris.get(key) || []) this.diagnostics.delete(vscode.Uri.parse(uri));
    this.diagnosticUris.delete(key);
    this.presentations.delete(key);
    this.refreshVisibleEditors();
  }

  refreshVisibleEditors() {
    for (const editor of vscode.window.visibleTextEditors) this.applyEditor(editor);
  }

  applyEditor(editor) {
    const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
    const entry = folder && this.presentations.get(folder.uri.toString());
    if (!entry) {
      editor.setDecorations(this.failures, []);
      editor.setDecorations(this.values, []);
      editor.setDecorations(this.coverage, []);
      return;
    }
    const relative = relativeDocumentPath(folder, editor.document);
    if (!relative) return;
    const lineRange = (line) => {
      const index = Math.max(0, Math.min(editor.document.lineCount - 1, line - 1));
      return editor.document.lineAt(index).range;
    };
    const failures = entry.presentation.diagnostics
      .filter((item) => item.path === relative)
      .map((item) => ({
        range: lineRange(item.line),
        hoverMessage: item.message,
        renderOptions: {
          after: {
            contentText: `  ← ${item.message}`,
            color: new vscode.ThemeColor(
              item.severity === "warning" ? "editorWarning.foreground" : "editorError.foreground",
            ),
            fontStyle: "italic",
          },
        },
      }));
    const values = entry.presentation.inlineValues
      .filter((item) => item.path === relative)
      .map((item) => ({
        range: lineRange(item.line),
        hoverMessage: item.hover,
        renderOptions: {
          after: {
            contentText: `  ${item.label}`,
            color: new vscode.ThemeColor("editorCodeLens.foreground"),
            fontStyle: "italic",
          },
        },
      }));
    const coverage = entry.presentation.coveredLines
      .filter((item) => item.path === relative)
      .map((item) => ({ range: lineRange(item.line), hoverMessage: `Covered by ${item.testId}` }));
    editor.setDecorations(this.failures, failures);
    editor.setDecorations(this.values, values);
    editor.setDecorations(this.coverage, coverage);
  }
}

class LiveWorkspace {
  constructor(folder, provider, decorations, publishResult) {
    this.folder = folder;
    this.provider = provider;
    this.decorations = decorations;
    this.publishResult = publishResult;
    this.userPaused = false;
    this.suspensions = 0;
    this.cancelInFlight = undefined;
    this.scheduler = new LiveScheduler({
      delay: configuration(folder).get("live.debounceMilliseconds", 500),
      run: (snapshot, signal, reason) => this.verify(snapshot, signal, reason),
      cancelRemote: () => this.cancelRemote(),
      onState: (state) => this.onState(state),
      onResult: (result, timing) => this.onResult(result, timing),
      onError: (error) => this.provider.write(this.folder, "live error", error.message),
    });
    this.watchers = [
      "**/src/**/*",
      "**/*.{java,kt,kts,gradle}",
      "**/{pom.xml,gradle.properties,libs.versions.toml,maven.config,jvm.config,extensions.xml,gradle-wrapper.properties}",
    ].map((pattern) => vscode.workspace.createFileSystemWatcher(
      new vscode.RelativePattern(folder, pattern),
    ));
    this.watcherDisposables = this.watchers.flatMap((watcher) => [
      watcher.onDidCreate((uri) => this.onWatchedFile(uri, "file created")),
      watcher.onDidChange((uri) => this.onWatchedFile(uri, "file saved")),
      watcher.onDidDelete((uri) => this.onWatchedFile(uri, "file deleted")),
    ]);
    this.configure();
  }

  dispose() {
    const cancelWorker = vscode.workspace.isTrusted && this.scheduler.hasActiveWork();
    this.scheduler.dispose();
    if (cancelWorker) this.cancelRemote();
    for (const watcher of this.watchers) watcher.dispose();
    for (const disposable of this.watcherDisposables) disposable.dispose();
    this.decorations.clear(this.folder);
  }

  configure() {
    this.scheduler.setDelay(configuration(this.folder).get("live.debounceMilliseconds", 500));
    if (
      this.mode() === "off" ||
      this.userPaused ||
      this.suspensions > 0 ||
      !vscode.workspace.isTrusted
    ) {
      this.scheduler.pause();
    } else {
      this.scheduler.resume();
    }
  }

  mode() {
    return configuration(this.folder).get("live.mode", "automatic");
  }

  onDocumentChanged(document) {
    if (!this.supportsDocument(document)) return;
    this.decorations.clear(this.folder);
    if (this.mode() === "automatic") this.schedule("typing");
    else if (this.mode() === "onSave" && !this.userPaused) {
      this.provider.setLiveState(this.folder, { state: "dirty", reason: "waiting for save" });
    }
  }

  onDocumentSaved(document) {
    if (this.mode() !== "off" && this.supportsDocument(document)) this.schedule("save");
  }

  onDocumentClosed(document) {
    if (!this.supportsDocument(document)) return;
    this.decorations.clear(this.folder);
    if (this.mode() !== "off") this.schedule("buffer closed");
  }

  schedule(reason) {
    if (!vscode.workspace.isTrusted || this.mode() === "off" || this.userPaused) return;
    this.scheduler.schedule(this.snapshot(), reason);
  }

  onWatchedFile(uri, reason) {
    const relative = relativeUriPath(this.folder, uri);
    if (relative && !ignoredWorkspacePath(relative)) this.schedule(reason);
  }

  supportsDocument(document) {
    if (!supportedDocument(document)) return false;
    const relative = relativeDocumentPath(this.folder, document);
    return Boolean(relative && !ignoredWorkspacePath(relative));
  }

  async runNow() {
    if (this.mode() === "off") {
      vscode.window.showInformationMessage("Enable walaru.live.mode to run live verification.");
      return;
    }
    const restorePause = this.userPaused;
    if (restorePause) {
      this.userPaused = false;
      this.configure();
    }
    this.schedule("manual");
    try {
      return await this.scheduler.flush();
    } finally {
      if (restorePause) {
        this.userPaused = true;
        this.configure();
      }
    }
  }

  start() {
    if (!vscode.workspace.isTrusted || this.mode() === "off") return undefined;
    this.schedule("initial");
    return this.scheduler.flush();
  }

  pause() {
    this.userPaused = true;
    this.scheduler.pause();
  }

  resume() {
    this.userPaused = false;
    this.configure();
    this.schedule("resume");
  }

  suspend() {
    this.suspensions += 1;
    this.scheduler.pause();
  }

  release() {
    this.suspensions = Math.max(0, this.suspensions - 1);
    this.configure();
  }

  snapshot() {
    const documents = [];
    for (const document of vscode.workspace.textDocuments) {
      if (!document.isDirty || !this.supportsDocument(document)) continue;
      if (vscode.workspace.getWorkspaceFolder(document.uri)?.uri.toString() !== this.folder.uri.toString()) continue;
      const relative = relativeDocumentPath(this.folder, document);
      if (!relative) continue;
      documents.push({ path: relative, version: document.version, content: document.getText() });
    }
    return { documents };
  }

  async verify(snapshot, signal, reason) {
    const result = await this.queryVerification(
      ["verify", "--supersede"],
      { signal, maxBytes: 1024 * 1024 },
      snapshot,
    );
    if (result.envelope.data?.cancelled) throw abortedRequest();
    const failureIds = Array.isArray(result.envelope.data?.failures)
      ? result.envelope.data.failures.slice(0, 20)
      : [];
    const failures = await Promise.all(
      failureIds.map(async (failureId) => (await runQuery(this.folder, ["failure", failureId], { signal })).envelope),
    );
    const coveragePaths = new Set(snapshot.documents.map((document) => document.path));
    const active = vscode.window.activeTextEditor?.document;
    if (active && vscode.workspace.getWorkspaceFolder(active.uri)?.uri.toString() === this.folder.uri.toString()) {
      const relative = relativeDocumentPath(this.folder, active);
      if (relative) coveragePaths.add(relative);
    }
    // A compiler/build failure has no fresh runtime evidence. Do not paint historical
    // coverage over the broken buffer as if it came from this verification.
    const coverage = result.envelope.data?.events > 0
      ? await Promise.all(
        Array.from(coveragePaths).slice(0, 8).map(async (subject) => (
          await runQuery(this.folder, ["coverage", subject], { signal, maxBytes: 256 * 1024 })
        ).envelope),
      )
      : [];
    return {
      ...livePresentation(result.envelope, failures, coverage),
      reason,
      stderr: result.stderr,
    };
  }

  async queryVerification(args, options = {}, snapshot = this.snapshot()) {
    if (this.cancelInFlight) await this.cancelInFlight;
    if (options.signal?.aborted) throw abortedRequest();
    // Clean saves, manual runs, and unsaved edits all use one isolated mirror so the
    // build daemon stays warm and no editor buffer is written into the real worktree.
    const temporary = await writeOverlayManifest(overlayManifest("vscode", snapshot.documents));
    const supersedingArgs = args.includes("--supersede")
      ? args
      : [args[0], "--supersede", ...args.slice(1)];
    try {
      const result = await runQuery(
        this.folder,
        [...supersedingArgs, "--overlay-manifest", temporary.manifest],
        options,
      );
      const hasOverlayVersions = Object.hasOwn(
        result.envelope.data || {},
        "overlayVersions",
      );
      if (
        !result.envelope.data?.cancelled &&
        (hasOverlayVersions || ["ok", "failure"].includes(result.envelope.status)) &&
        !overlayVersionsMatch(snapshot.documents, result.envelope.data?.overlayVersions)
      ) {
        throw new Error("Walaru returned editor versions that do not match the requested snapshot");
      }
      return result;
    } finally {
      await temporary.dispose();
    }
  }

  cancelRemote() {
    if (this.cancelInFlight) return this.cancelInFlight;
    this.cancelInFlight = runQuery(this.folder, ["cancel"])
      .catch(() => undefined)
      .finally(() => {
        this.cancelInFlight = undefined;
      });
    return this.cancelInFlight;
  }

  onState(state) {
    if (state.state === "queued") this.decorations.clear(this.folder);
    this.provider.setLiveState(this.folder, state);
  }

  onResult(result, timing) {
    this.decorations.apply(this.folder, result);
    this.provider.write(this.folder, "live", formatLiveResult(result, timing.elapsedMs));
    if (result.stderr?.trim()) this.provider.write(this.folder, "live stderr", result.stderr.trimEnd());
    this.provider.applyLiveResult(this.folder, result);
    this.publishResult(this.folder, result, timing);
    this.provider.refresh(this.folder);
  }
}

class LiveManager {
  constructor(provider, decorations) {
    this.provider = provider;
    this.decorations = decorations;
    this.workspaces = new Map();
    this.resultListener = () => {};
    this.syncFolders();
  }

  dispose() {
    for (const workspace of this.workspaces.values()) workspace.dispose();
    this.workspaces.clear();
  }

  syncFolders() {
    const active = new Set((vscode.workspace.workspaceFolders || []).map((folder) => folder.uri.toString()));
    for (const [key, workspace] of this.workspaces) {
      if (!active.has(key)) {
        workspace.dispose();
        this.provider.removeWorkspace(workspace.folder);
        this.workspaces.delete(key);
      }
    }
    for (const folder of vscode.workspace.workspaceFolders || []) {
      const key = folder.uri.toString();
      if (!this.workspaces.has(key)) {
        this.workspaces.set(
          key,
          new LiveWorkspace(
            folder,
            this.provider,
            this.decorations,
            (workspace, result, timing) => this.resultListener(workspace, result, timing),
          ),
        );
      }
    }
  }

  workspaceFor(documentOrFolder) {
    const folder = documentOrFolder?.uri && documentOrFolder.name
      ? documentOrFolder
      : vscode.workspace.getWorkspaceFolder(documentOrFolder?.uri);
    return folder && this.workspaces.get(folder.uri.toString());
  }

  changed(event) {
    if (event.contentChanges?.length > 0) this.workspaceFor(event.document)?.onDocumentChanged(event.document);
  }

  saved(document) {
    this.workspaceFor(document)?.onDocumentSaved(document);
  }

  closed(document) {
    this.workspaceFor(document)?.onDocumentClosed(document);
  }

  configure() {
    for (const workspace of this.workspaces.values()) workspace.configure();
  }

  pause(folder) {
    if (folder) this.workspaceFor(folder)?.pause();
    else for (const workspace of this.workspaces.values()) workspace.pause();
  }

  resume(folder) {
    if (folder) this.workspaceFor(folder)?.resume();
    else for (const workspace of this.workspaces.values()) workspace.resume();
  }

  suspend(folder) {
    this.workspaceFor(folder)?.suspend();
  }

  release(folder) {
    this.workspaceFor(folder)?.release();
  }

  run(folder) {
    return this.workspaceFor(folder)?.runNow();
  }

  start(folder) {
    return this.workspaceFor(folder)?.start();
  }

  verify(folder, args, options = {}) {
    const workspace = this.workspaceFor(folder);
    if (!workspace) throw new Error("Walaru workspace is unavailable");
    return workspace.queryVerification(args, options);
  }

  setResultListener(listener) {
    this.resultListener = listener;
  }
}

function configuration(folder) {
  return vscode.workspace.getConfiguration("walaru", folder.uri);
}

function runQuery(folder, args, options = {}) {
  const binaryPath = configuration(folder).get("binaryPath", "");
  return client.query(folder.uri.fsPath, args, { binaryPath, ...options });
}

function emptyModel(folder) {
  return { folder, workspaceId: "", revision: "", daemonRunning: false, tests: [], error: null };
}

function supportedDocument(document) {
  if (document.uri.scheme !== "file") return false;
  if (["java", "kotlin", "groovy"].includes(document.languageId)) return true;
  const normalized = document.uri.fsPath.replaceAll(path.sep, "/");
  const name = path.basename(document.uri.fsPath);
  return /\.(java|kt|kts|gradle)$/.test(normalized)
    || normalized.includes("/src/")
    || BUILD_INPUT_NAMES.has(name);
}

function relativeDocumentPath(folder, document) {
  if (document.uri.scheme !== "file") return undefined;
  return relativeUriPath(folder, document.uri);
}

function relativeUriPath(folder, uri) {
  if (uri.scheme !== "file") return undefined;
  const relative = path.relative(folder.uri.fsPath, uri.fsPath).replaceAll(path.sep, "/");
  if (!relative || relative === ".." || relative.startsWith("../") || path.posix.isAbsolute(relative)) {
    return undefined;
  }
  return relative;
}

async function writeOverlayManifest(manifest) {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "walaru-overlay-"));
  const manifestPath = path.join(directory, "overlay.json");
  await fs.writeFile(manifestPath, JSON.stringify(manifest), { encoding: "utf8", mode: 0o600 });
  return {
    manifest: manifestPath,
    dispose: () => fs.rm(directory, { recursive: true, force: true }).catch(() => undefined),
  };
}

function abortedRequest() {
  const error = new Error("Walaru live request was superseded");
  error.name = "AbortError";
  return error;
}

function formatLiveResult(result, elapsedMs) {
  const lines = [
    `Walaru live · ${result.status} · ${(elapsedMs / 1000).toFixed(2)}s`,
    `Revision: ${result.revision}`,
    `Selection: ${result.selectedTests.length > 0 ? result.selectedTests.join(", ") : "module fallback"}`,
    `Problems: ${result.diagnostics.length}; values: ${result.inlineValues.length}; covered lines: ${result.coveredLines.length}`,
  ];
  for (const diagnostic of result.diagnostics.slice(0, 20)) {
    lines.push(`  ${diagnostic.path}:${diagnostic.line}:${diagnostic.column} ${diagnostic.message}`);
  }
  return lines.join("\n");
}

async function chooseFolder(node) {
  if (node?.folder) return node.folder;
  const folders = vscode.workspace.workspaceFolders || [];
  if (folders.length === 0) throw new Error("Open a workspace before running Walaru");
  if (folders.length === 1) return folders[0];
  const selected = await vscode.window.showQuickPick(
    folders.map((folder) => ({ label: folder.name, description: folder.uri.fsPath, folder })),
    { placeHolder: "Choose a Walaru workspace" },
  );
  return selected?.folder;
}

async function chooseTest(provider, node) {
  if (node?.test) return node;
  const tests = provider.allTests();
  const selected = await vscode.window.showQuickPick(
    tests.map((item) => ({
      label: item.test.displayName,
      description: `${item.folder.name} · ${item.test.module} · ${item.test.status}`,
      item,
    })),
    { placeHolder: "Choose a Walaru test", matchOnDescription: true },
  );
  return selected?.item;
}

function requireTrust() {
  if (model.executionAllowed(vscode.workspace.isTrusted)) return true;
  vscode.window.showWarningMessage("Walaru does not execute binaries in an untrusted workspace.");
  return false;
}

async function activate(context) {
  const output = vscode.window.createOutputChannel("Walaru");
  const statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
  statusBar.command = "walaru.runLive";
  const provider = new WalaruTreeProvider(output, statusBar);
  const decorations = new LiveDecorations(context);
  const live = new LiveManager(provider, decorations);
  const testIntegration = new WalaruTestIntegration(provider, live, output);
  live.setResultListener((folder, result, timing) => {
    testIntegration.publishLive(folder, result, timing);
  });
  provider.onModelChange = () => testIntegration.sync();
  const view = vscode.window.createTreeView("walaru.tests", { treeDataProvider: provider });

  const runWorkspaceCommand = async (command, node) => {
    if (!requireTrust()) return;
    let folder;
    const exclusive = command === "verify" || command === "fullVerify";
    try {
      folder = await chooseFolder(node);
      if (!folder) return;
      if (exclusive) live.suspend(folder);
      const args = model.commandArguments(command);
      const result = exclusive
        ? await live.verify(folder, args, { maxBytes: 1024 * 1024 })
        : await runQuery(folder, args);
      provider.write(folder, args[0], model.formatEnvelope(args[0], result.envelope));
      if (result.stderr) provider.write(folder, "stderr", result.stderr.trimEnd());
      output.show(true);
      if (result.exitCode !== 0) vscode.window.showWarningMessage(`Walaru exited with ${result.exitCode}`);
      await provider.refresh(folder);
    } catch (error) {
      vscode.window.showErrorMessage(`Walaru: ${error.message}`);
    } finally {
      if (folder && exclusive) live.release(folder);
    }
  };

  const runTestCommand = async (command, node) => {
    if (!requireTrust()) return;
    let selected;
    const exclusive = command === "record";
    try {
      selected = await chooseTest(provider, node);
      if (!selected) return;
      if (exclusive) live.suspend(selected.folder);
      const args = model.commandArguments(command, selected.test);
      const result = await runQuery(selected.folder, args);
      provider.write(selected.folder, args[0], model.formatEnvelope(args[0], result.envelope));
      if (result.stderr) provider.write(selected.folder, "stderr", result.stderr.trimEnd());
      output.show(true);
      if (result.exitCode !== 0) vscode.window.showWarningMessage(`Walaru exited with ${result.exitCode}`);
      await provider.refresh(selected.folder);
    } catch (error) {
      vscode.window.showErrorMessage(`Walaru: ${error.message}`);
    } finally {
      if (selected && exclusive) live.release(selected.folder);
    }
  };

  const subscriptions = [
    vscode.commands.registerCommand("walaru.refresh", () => provider.refreshAll()),
    vscode.commands.registerCommand("walaru.status", (node) => runWorkspaceCommand("status", node)),
    vscode.commands.registerCommand("walaru.doctor", (node) => runWorkspaceCommand("doctor", node)),
    vscode.commands.registerCommand("walaru.verify", (node) => runWorkspaceCommand("verify", node)),
    vscode.commands.registerCommand("walaru.fullVerify", (node) => runWorkspaceCommand("fullVerify", node)),
    vscode.commands.registerCommand("walaru.failure", (node) => runTestCommand("failure", node)),
    vscode.commands.registerCommand("walaru.trace", (node) => runTestCommand("trace", node)),
    vscode.commands.registerCommand("walaru.record", (node) => runTestCommand("record", node)),
    vscode.commands.registerCommand("walaru.runLive", async (node) => {
      if (!requireTrust()) return;
      const folder = await chooseFolder(node);
      if (folder) await live.run(folder);
    }),
    vscode.commands.registerCommand("walaru.pauseLive", async (node) => {
      const folder = await chooseFolder(node);
      if (folder) live.pause(folder);
    }),
    vscode.commands.registerCommand("walaru.resumeLive", async (node) => {
      const folder = await chooseFolder(node);
      if (folder) live.resume(folder);
    }),
    vscode.commands.registerCommand("walaru.openTui", async (node) => {
      if (!requireTrust()) return;
      try {
        const folder = await chooseFolder(node);
        if (!folder) return;
        const binaryPath = configuration(folder).get("binaryPath", "").trim() || "walaru";
        const terminal = vscode.window.createTerminal({
          name: `Walaru · ${folder.name}`,
          cwd: folder.uri,
          shellPath: binaryPath,
          shellArgs: ["--workspace", folder.uri.fsPath, "tui"],
        });
        terminal.show();
      } catch (error) {
        vscode.window.showErrorMessage(`Walaru: ${error.message}`);
      }
    }),
    view.onDidChangeVisibility((event) => provider.setVisible(event.visible)),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.contentChanges?.length > 0 && supportedDocument(event.document)) {
        testIntegration.invalidate();
      }
      live.changed(event);
    }),
    vscode.workspace.onDidSaveTextDocument((document) => live.saved(document)),
    vscode.workspace.onDidCloseTextDocument((document) => live.closed(document)),
    vscode.window.onDidChangeVisibleTextEditors(() => decorations.refreshVisibleEditors()),
    vscode.window.onDidChangeActiveTextEditor(() => decorations.refreshVisibleEditors()),
    vscode.workspace.onDidChangeWorkspaceFolders((event) => {
      provider.syncWorkspaceFolders();
      live.syncFolders();
      provider.startTimers();
      if (provider.visible) provider.refreshAll();
      if (vscode.workspace.isTrusted) {
        for (const folder of event.added) live.start(folder);
      }
    }),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("walaru")) {
        provider.startTimers();
        live.configure();
        if (provider.visible) provider.refreshAll();
        if (
          vscode.workspace.isTrusted &&
          (event.affectsConfiguration("walaru.live.mode") ||
            event.affectsConfiguration("walaru.binaryPath"))
        ) {
          for (const folder of vscode.workspace.workspaceFolders || []) live.start(folder);
        }
      }
    }),
  ];
  if (vscode.workspace.onDidGrantWorkspaceTrust) {
    subscriptions.push(vscode.workspace.onDidGrantWorkspaceTrust(() => {
      live.configure();
      provider.setVisible(view.visible);
      if (!view.visible) provider.refreshAll();
      for (const folder of vscode.workspace.workspaceFolders || []) live.start(folder);
    }));
  }
  context.subscriptions.push(
    output,
    statusBar,
    provider,
    decorations,
    live,
    testIntegration,
    view,
    ...subscriptions,
  );
  provider.updateStatus();
  provider.setVisible(view.visible);
  if (vscode.workspace.isTrusted) {
    if (!view.visible) provider.refreshAll();
    for (const folder of vscode.workspace.workspaceFolders || []) live.start(folder);
  }
}

function deactivate() {}

module.exports = { activate, deactivate };
