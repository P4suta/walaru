import { readFile } from "node:fs/promises";

const manifestUrl = new URL("../package.json", import.meta.url);
const manifest = JSON.parse(await readFile(manifestUrl, "utf8"));
const commands = new Set(manifest.contributes?.commands?.map((item) => item.command));
const required = [
  "walaru.refresh",
  "walaru.status",
  "walaru.doctor",
  "walaru.verify",
  "walaru.fullVerify",
  "walaru.failure",
  "walaru.trace",
  "walaru.record",
  "walaru.openTui",
];
for (const command of required) {
  if (!commands.has(command)) throw new Error(`missing command ${command}`);
}
if (manifest.engines?.vscode !== "^1.95.0") throw new Error("unexpected VS Code engine");
if (manifest.contributes?.configuration?.properties?.["walaru.binaryPath"]?.scope !== "resource") {
  throw new Error("walaru.binaryPath must be resource scoped");
}
if (manifest.contributes?.configuration?.properties?.["walaru.refreshIntervalSeconds"]?.scope !== "resource") {
  throw new Error("walaru.refreshIntervalSeconds must be resource scoped");
}
if (!manifest.contributes?.viewsContainers?.activitybar?.some((item) => item.id === "walaru")) {
  throw new Error("missing Walaru activity bar container");
}
process.stdout.write("VS Code manifest is valid\n");
