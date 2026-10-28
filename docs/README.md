# Walaru

Walaru connects Java and Kotlin tests to local failure analysis, impact-based verification, and replay in a fresh Java virtual machine.
Add one Gradle plugin and use the zero-dependency API to capture state.
A failing test produces an offline explanation, source focus, safe values, and a self-contained report.

```java
try (var span = Walaru.span("binary search").capture("target", target)) {
    int middle = Walaru.capture("middle", (low + high) >>> 1);
    Walaru.checkpoint("partition", Map.of("low", low, "high", high));
    Walaru.captureRedacted("apiToken", token);
}
```

Without the agent, every call is fail-open: values return unchanged and spans do nothing.
Failure analysis has no telemetry, AI, or network dependency.

## Try it from source

Use Java Development Kit 21 or later for the Java example.
Build the standalone command-line tools with the Rust toolchain in `rust-toolchain.toml`.
Build the VS Code extension with Node.js 22 or later.
Pull requests run the ordinary checks on Linux with Java 21.
Release checks cover Java 21 and 25, Gradle and Maven, JUnit Platform including Kotest, TestNG, Linux x64, macOS ARM64 and Intel, and Windows x64.
The repository contains intentionally failing Java and Kotlin examples.

```bash
git clone https://github.com/P4suta/walaru.git
cd walaru/examples/java-library-first
../../gradlew walaruExplain       # expected exit: test failure
```

Open `build/reports/walaru/index.html`.
The report contains the assertion difference, likely cause, focused source line, named values, and next steps.
It represents the secret as `<redacted>`.
The [Kotlin example](../examples/kotlin-library-first) demonstrates the same workflow.

Build the local VS Code extension package for editor feedback.
Open the Java example.
Walaru verifies unsaved buffers in an isolated mirror, cancels obsolete runs while typing, and reports compiler and test failures, captured values, coverage, and current test state in VS Code.
See the [VS Code guide](../clients/vscode/README.md) and [Java walkthrough](../examples/java-library-first/README.md).

To use the source plugin in another build, add the checkout once.

```kotlin
// settings.gradle.kts
pluginManagement { includeBuild("/path/to/walaru") }

// build.gradle.kts
plugins {
    java // or kotlin("jvm")
    id("io.github.p4suta.walaru")
}
```

The plugin supplies the API, agent, JUnit and TestNG lifecycle adapter, and JSON, Markdown, and HTML reports.
`test`, `walaruVerify`, `walaruExplain`, and `walaruReport` work without listener or Java virtual machine argument configuration.
The [library guide](library-api.md) also documents local Maven publication with `./gradlew publishToMavenLocal -Dmaven.repo.local="$PWD/.m2" --no-daemon`.

## One-command diagnosis and replay

Build the standalone runtime, then ask Walaru to verify, explain bounded failures, and record failing tests within the requested count and shared recording-time budget.

```bash
cd /path/to/walaru
./gradlew :jvm-agent:fatJar :jvm-runner:fatJar :gradle-adapter:fatJar --no-daemon
cargo build -p walaru-cli

target/debug/walaru --workspace examples/java-library-first --format human explain
```

`explain` exits `1` when tests fail while returning analysis and recording IDs.
Follow a recording with `replay` or `reverse`.
Replay compares recorded events through the requested target in a fresh Java virtual machine.
Only matching results return `verified: true`.
Use `tests`, `failure`, `trace`, or the interactive `tui` to inspect local evidence.

## Library surfaces

`walaru-api` provides capture, checkpoint, span, lazy diagnostic, redaction, and asynchronous context propagation for production Java and Kotlin code without runtime dependencies.
The Gradle plugin supplies the embedded runtime, framework discovery, agent wiring, impact model, and local reports.
`walaru-testkit` supplies ServiceLoader adapters for JUnit Platform and TestNG when wiring Maven or a custom launcher directly.
`walaru-client` provides a typed, bounded Java and Kotlin client that launches commands without a shell.
The command-line interface exposes JSON and newline-delimited JSON with schema version `1` for the terminal user interface and editor integrations.

See the complete [library guide](library-api.md), [public contracts](contracts.md), [architecture](architecture.md), and [replay contract](replay.md).

## Safety contract

Safe capture avoids user getters and `toString()`.
It bounds depth, strings, collections, event lines, query pages, logs, and responses.
Walaru redacts names and messages that look like secrets.
`captureRedacted` never gives the value to the agent.
Results bind to a content revision.
A concurrent edit makes an in-flight result stale.
Unknown dependencies and API, resource, or build changes conservatively widen test selection.
Network access, subprocesses, the Java Native Interface, default file I/O, ambiguous scheduling, and unsupported inputs make a recording partial.
Exit `4` never represents exact success.
State stays under `.gradle/walaru/<workspace-id>`.

## Development status

Walaru remains unreleased.
Build the project from source for local use.

```bash
scripts/check.sh
cargo test --workspace
./gradlew check --no-daemon
npm --prefix clients/vscode test
```

See [CONTRIBUTING.md](../CONTRIBUTING.md).
Walaru uses the existing [Massachusetts Institute of Technology (MIT) license](../LICENSE-MIT) or [Apache License, Version 2.0](../LICENSE-APACHE), at your option.
[NOTICE](../NOTICE) preserves the third-party Zstandard attribution.
