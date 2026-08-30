# Walaru for VS Code

The bundled extension shows Walaru workspaces and failure-first tests in the Activity Bar. It can open failures and traces, record a selected test, run impacted or full verification, and launch the interactive TUI.

The extension executes no external binary until the workspace is trusted. `walaru.binaryPath` and `walaru.refreshIntervalSeconds` are resource-scoped for multi-root workspaces. Automatic refresh runs only while the view is visible.

This source package is not published to the VS Code Marketplace. Build a local VSIX with `npm ci && npm run package`.
