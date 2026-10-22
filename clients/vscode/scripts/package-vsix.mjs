import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { createVSIX } from "@vscode/vsce";
import manifest from "../package.json" with { type: "json" };

const extensionRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(extensionRoot, "../..");
const outputDirectory = path.join(repositoryRoot, "dist");
await mkdir(outputDirectory, { recursive: true });
const output = path.join(outputDirectory, `walaru-${manifest.version}.vsix`);
await createVSIX({
  cwd: extensionRoot,
  packagePath: output,
});
process.stdout.write(`${output}\n`);
