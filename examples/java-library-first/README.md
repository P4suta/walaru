# Java example

Run:

```bash
../../gradlew walaruExplain
```

The binary search intentionally fails. Walaru returns the original Gradle failure status and writes
`build/reports/walaru/index.html`, `report.md`, and `report.json`. The report links the failure to the
last partition, shows named values, and contains `<redacted>` rather than the API token.

For a full ordered recording after building the CLI, run from the repository root:

```bash
target/debug/walaru --workspace examples/java-library-first --format human explain
```

## Edit it live in VS Code

From the repository root, build the runtime, CLI, and local extension:

```bash
./gradlew :jvm-agent:fatJar :jvm-runner:fatJar :gradle-adapter:fatJar --no-daemon
cargo build -p walaru-cli
npm --prefix clients/vscode ci
npm --prefix clients/vscode run package
code --install-extension dist/walaru-0.1.0.vsix --force
code examples/java-library-first
```

This example preconfigures `walaru.binaryPath` as `../../target/debug/walaru`. With live mode left at
its `automatic` default, change `while (low < high)` to `while (low <= high)` without saving. The
failure should disappear after the warm verification. Introduce a Java syntax error to see a
source-linked Problems entry, then undo it. The `middle` and `partition` captures update beside
their source lines; the file on disk remains the intentionally failing example throughout.
