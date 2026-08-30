# Walaru

Walaru is local-first test intelligence and deterministic replay for Java and Kotlin. A stable JSON API is the product core; the CLI, interactive TUI, VS Code view, IntelliJ tools, and agent skill are clients of the same revision-bound evidence.

Walaru answers four practical questions without changing the target project:

- Which tests are affected by this edit?
- What failed, where, and with which safely captured values?
- What did one test execute, in order?
- Can a recorded state be reproduced and verified in a fresh JVM?

## Supported environments

- JDK 21 or newer; CI covers JDK 21 and 25.
- Gradle 9.x through the Wrapper, or Maven Surefire.
- JUnit Platform (including Jupiter and Kotest) and TestNG.
- Java, Kotlin, mixed-language, and multi-module workspaces.
- Linux x64, macOS ARM64/Intel, and Windows x64.

This repository is pre-release source. No crates.io, Maven Central, VS Code Marketplace, tag, or GitHub Release has been published yet.

## Build from source

Install Rust 1.97, JDK 21+, and Node.js 22+, then run:

```bash
export GRADLE_USER_HOME="$PWD/.gradle-user-home"
./gradlew :jvm-agent:fatJar :jvm-runner:fatJar :gradle-adapter:fatJar --no-daemon
cargo build -p walaru-cli
```

The CLI is `target/debug/walaru`. Runtime JAR discovery uses the Cargo workspace version, so normal version bumps do not require hand-edited artifact paths.

## Five-minute demo

The included Gradle fixture contains an intentional failure:

```bash
WALARU=target/debug/walaru

$WALARU --workspace fixtures/mixed-gradle --format human doctor
$WALARU --workspace fixtures/mixed-gradle --format human verify
$WALARU --workspace fixtures/mixed-gradle --format human tests
```

`verify` exits `1` for the intentional test failure. `tests` includes `lastFailureId`, which can be opened directly:

```bash
$WALARU --workspace fixtures/mixed-gradle --format human failure <failure-id>
$WALARU --workspace fixtures/mixed-gradle --format human trace <test-id>
$WALARU --workspace fixtures/mixed-gradle --format human record <test-id>
$WALARU --workspace fixtures/mixed-gradle --format human reverse <recording-id> --from <event> --step line
$WALARU --workspace fixtures/mixed-gradle tui
```

The `record` and `reverse` commands expose deterministic fresh-JVM replay and earlier event boundaries without pretending that a partial recording is verified.

Agents and integrations should request the structured contract explicitly:

```bash
$WALARU --workspace fixtures/mixed-gradle --format json --limit 100 tests
$WALARU --workspace fixtures/mixed-gradle --format json \
  --fields events.id,events.sequence,events.kind,events.location trace <test-id>
```

Every finite command returns envelope schema version `1`. Exit `4` means stale, truncated, partial, unsupported, or unverified; it is never an exact success.

## Guarantees and limits

- Observations are bound to a content revision. A workspace edit makes an in-flight result stale.
- Responses, event lines, logs, pages, values, strings, fields, and arrays are bounded.
- Value capture does not invoke user getters or `toString()` and redacts secret-looking data.
- Unknown dependencies and API, resource, or build changes conservatively widen test selection.
- `verified: true` requires a complete recording and a matching fresh-JVM replay prefix.
- Network, subprocess, JNI, default file I/O, ambiguous thread identity, and unsupported inputs make a recording partial.
- State remains under `.gradle/walaru/<workspace-id>`; there is no telemetry.

See [public contracts](docs/contracts.md), [architecture](docs/architecture.md), and [replay guarantees](docs/replay.md).

## Development

```bash
scripts/check.sh
cargo test --workspace
./gradlew check --no-daemon
npm --prefix clients/vscode test
npm --prefix clients/vscode run validate
```

Contributions use test-first contracts; see [CONTRIBUTING.md](CONTRIBUTING.md). Walaru is dual-licensed under Apache-2.0 or MIT, at your option.
