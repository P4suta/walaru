# Walaru for VS Code

Walaru continuously verifies Java, Kotlin, Gradle, and Maven edits, including unsaved buffers. After
the debounce window it cancels obsolete work, runs conservatively impacted tests in a private
workspace mirror, and rejects results that do not match the latest editor generation.

Current feedback appears where it is useful:

- compiler and test problems in Problems, on the source line, and in the overview ruler;
- safely captured values after their source line (including compact loop value transitions) and
  coverage markers in the gutter;
- queued/running/pass/fail state in the Status Bar;
- live and manual results in VS Code Testing, including exact single-test runs;
- failure-first workspaces and tests in the Walaru Activity Bar.

`Walaru: Pause Live Verification`, `Resume Live Verification`, and `Run Live Verification Now`
control the loop. `walaru.live.mode` is `automatic`, `onSave`, or `off`; the default debounce is 500
ms. All settings are resource-scoped, so multi-root workspaces may use different binaries and
policies.

Unsaved text never replaces the real file. One request accepts at most 256 documents, 1 MiB per
document, and 4 MiB total, then executes the snapshot below `.gradle/walaru/`. A new edit clears old
feedback immediately. Workspace Trust is required before any external binary runs.

This source package is not published to the VS Code Marketplace. Build and install it locally:

```bash
npm ci
npm run package
code --install-extension ../../dist/walaru-0.1.0.vsix --force
```
