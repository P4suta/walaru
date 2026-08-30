import { mkdir, readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const extensionRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(extensionRoot, "../..");
const manifest = JSON.parse(await readFile(path.join(extensionRoot, "package.json"), "utf8"));
const outputDirectory = path.join(repositoryRoot, "dist");
await mkdir(outputDirectory, { recursive: true });
const output = path.join(outputDirectory, `walaru-${manifest.version}.vsix`);
const executable = process.platform === "win32" ? "npx.cmd" : "npx";
const result = spawnSync(executable, ["--no-install", "vsce", "package", "--out", output], {
  cwd: extensionRoot,
  stdio: "inherit",
  shell: false,
});
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
process.stdout.write(`${output}\n`);
