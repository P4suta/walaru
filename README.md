# Walaru

Walaru turns ordinary Java and Kotlin tests into useful, local evidence. Add one Gradle plugin, use a
tiny zero-dependency API where state matters, and a failing test produces an offline explanation,
source focus, safe values, and a self-contained report. The same evidence also powers impact-based
verification and verified fresh-JVM replay.

```java
try (var span = Walaru.span("binary search").capture("target", target)) {
    int middle = Walaru.capture("middle", (low + high) >>> 1);
    Walaru.checkpoint("partition", Map.of("low", low, "high", high));
    Walaru.captureRedacted("apiToken", token);
}
```

Without the agent, every call is fail-open: values are returned unchanged and spans are no-ops.
There is no telemetry and no AI or network dependency in failure analysis.

## Try it from source

Requirements are JDK 21+, Rust 1.97, and Node.js 22+. CI covers JDK 21/25, Gradle and Maven,
JUnit Platform (including Kotest), TestNG, Linux x64, macOS ARM64/Intel, and Windows x64.

The repository contains intentionally failing Java and Kotlin examples:

```bash
git clone https://github.com/P4suta/walaru.git
cd walaru/examples/java-library-first
../../gradlew walaruExplain       # expected exit: test failure
```

Open `build/reports/walaru/index.html`. It contains the assertion difference, likely cause, focused
source line, named values, and next steps; the secret is represented only as `<redacted>`. The
Kotlin example is at [`examples/kotlin-library-first`](examples/kotlin-library-first).

To use the source plugin in another build, add the checkout once:

```kotlin
// settings.gradle.kts
pluginManagement { includeBuild("/path/to/walaru") }

// build.gradle.kts
plugins {
    java // or kotlin("jvm")
    id("io.github.p4suta.walaru")
}
```

That one plugin supplies the API, agent, JUnit/TestNG lifecycle adapter, and JSON/Markdown/HTML
reports. `test`, `walaruVerify`, `walaruExplain`, and `walaruReport` work without listener or JVM
argument configuration.

## One-command diagnosis and replay

Build the standalone runtime, then ask Walaru to verify, explain bounded failures, and record as many
failed tests as fit the requested count and shared recording-time budget:

```bash
cd /path/to/walaru
./gradlew :jvm-agent:fatJar :jvm-runner:fatJar :gradle-adapter:fatJar --no-daemon
cargo build -p walaru-cli

target/debug/walaru --workspace examples/java-library-first --format human explain
```

`explain` deliberately exits `1` when tests fail while still returning usable analysis and recording
IDs. Follow one recording with `replay` or `reverse`; only a matching fresh JVM returns
`verified: true`. For quick inspection use `tests`, `failure`, `trace`, or the interactive `tui`.

## Library surfaces

- `walaru-api`: zero-dependency capture, checkpoint, span, lazy diagnostic, redaction, and async
  context propagation API for production Java/Kotlin code.
- Gradle plugin: embedded runtime, automatic framework discovery, safe agent wiring, impact model,
  and local reports from a single plugin declaration.
- `walaru-testkit`: ServiceLoader adapters for JUnit Platform and TestNG when wiring Maven or a
  custom launcher directly.
- `walaru-client`: typed, bounded, shell-free Java/Kotlin client for applications and build tools.
- CLI JSON/NDJSON schema `1`: stable automation boundary used by the TUI and editor integrations.

See the complete [library guide](docs/library-api.md), [public contracts](docs/contracts.md),
[architecture](docs/architecture.md), and [replay guarantees](docs/replay.md).

## Safety contract

- Safe capture does not invoke user getters or `toString()` and bounds depth, strings, collections,
  event lines, query pages, logs, and responses.
- Secret-looking names/messages are redacted; `captureRedacted` never gives the value to the agent.
- Results are bound to a content revision. A concurrent edit makes an in-flight result stale.
- Unknown dependencies and API/resource/build changes conservatively widen test selection.
- Network, subprocess, JNI, default file I/O, ambiguous scheduling, and unsupported inputs make a
  recording partial; exit `4` is never presented as exact success.
- State stays under `.gradle/walaru/<workspace-id>`.

## Development status

This is pre-release source. No tag, GitHub Release, crates.io artifact, Maven Central artifact, or VS
Code Marketplace entry has been published.

```bash
scripts/check.sh
cargo test --workspace
./gradlew check --no-daemon
npm --prefix clients/vscode test
```

See [CONTRIBUTING.md](CONTRIBUTING.md). Walaru is dual-licensed under Apache-2.0 or MIT.
