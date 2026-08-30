"use strict";

const vscode = require("vscode");
const client = require("./client");
const model = require("./model");

class WalaruTreeProvider {
  constructor(output, statusBar) {
    this.output = output;
    this.statusBar = statusBar;
    this.models = new Map();
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
      item.description = element.model.error
        ? "unavailable"
        : `${element.model.tests.length} tests`;
      item.contextValue = "walaruWorkspace";
      item.iconPath = new vscode.ThemeIcon(element.model.error ? "error" : "folder-library");
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
    return element.model.tests.map((test) => ({
      kind: "test",
      folder: element.folder,
      test,
    }));
  }

  allTests() {
    const items = [];
    for (const current of this.models.values()) {
      for (const test of current.tests) {
        items.push({ kind: "test", folder: current.folder, test });
      }
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

  startTimers() {
    this.stopTimers();
    if (!this.visible || !vscode.workspace.isTrusted) return;
    for (const folder of vscode.workspace.workspaceFolders || []) {
      const seconds = configuration(folder).get("refreshIntervalSeconds", 10);
      const interval = Math.max(2, Math.min(300, Number(seconds) || 10)) * 1_000;
      this.timers.set(
        folder.uri.toString(),
        setInterval(() => this.refresh(folder), interval),
      );
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
      this.updateStatus();
      this.emitter.fire();
    }
  }

  updateStatus() {
    const summary = model.statusSummary(this.models.values(), vscode.workspace.isTrusted);
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

function configuration(folder) {
  return vscode.workspace.getConfiguration("walaru", folder.uri);
}

function runQuery(folder, args) {
  const binaryPath = configuration(folder).get("binaryPath", "");
  return client.query(folder.uri.fsPath, args, { binaryPath });
}

function emptyModel(folder) {
  return {
    folder,
    workspaceId: "",
    revision: "",
    daemonRunning: false,
    tests: [],
    error: null,
  };
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
  statusBar.command = "walaru.refresh";
  const provider = new WalaruTreeProvider(output, statusBar);
  const view = vscode.window.createTreeView("walaru.tests", { treeDataProvider: provider });

  const runWorkspaceCommand = async (command, node) => {
    if (!requireTrust()) return;
    try {
      const folder = await chooseFolder(node);
      if (!folder) return;
      const args = model.commandArguments(command);
      const result = await runQuery(folder, args);
      provider.write(folder, args[0], model.formatEnvelope(args[0], result.envelope));
      if (result.stderr) provider.write(folder, "stderr", result.stderr.trimEnd());
      output.show(true);
      if (result.exitCode !== 0) {
        vscode.window.showWarningMessage(`Walaru exited with ${result.exitCode}`);
      }
      await provider.refresh(folder);
    } catch (error) {
      vscode.window.showErrorMessage(`Walaru: ${error.message}`);
    }
  };

  const runTestCommand = async (command, node) => {
    if (!requireTrust()) return;
    try {
      const selected = await chooseTest(provider, node);
      if (!selected) return;
      const args = model.commandArguments(command, selected.test);
      const result = await runQuery(selected.folder, args);
      provider.write(selected.folder, args[0], model.formatEnvelope(args[0], result.envelope));
      if (result.stderr) provider.write(selected.folder, "stderr", result.stderr.trimEnd());
      output.show(true);
      if (result.exitCode !== 0) {
        vscode.window.showWarningMessage(`Walaru exited with ${result.exitCode}`);
      }
      await provider.refresh(selected.folder);
    } catch (error) {
      vscode.window.showErrorMessage(`Walaru: ${error.message}`);
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
    vscode.workspace.onDidChangeWorkspaceFolders(() => {
      provider.syncWorkspaceFolders();
      provider.startTimers();
      if (provider.visible) provider.refreshAll();
    }),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("walaru")) {
        provider.startTimers();
        if (provider.visible) provider.refreshAll();
      }
    }),
  ];
  if (vscode.workspace.onDidGrantWorkspaceTrust) {
    subscriptions.push(vscode.workspace.onDidGrantWorkspaceTrust(() => provider.setVisible(view.visible)));
  }
  context.subscriptions.push(output, statusBar, provider, view, ...subscriptions);
  provider.updateStatus();
  provider.setVisible(view.visible);
}

function deactivate() {}

module.exports = { activate, deactivate };
