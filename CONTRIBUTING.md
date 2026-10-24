# Contributing

Public communication, code, tests, and documentation are in English.

## Development setup

Install the Rust toolchain pinned in `rust-toolchain.toml`, Java Development Kit (JDK) 21+, Node.js 22+, and a POSIX shell on Linux/macOS.
Windows packaging requires PowerShell 7.
Run the project checks:

```bash
export GRADLE_USER_HOME="$PWD/.gradle-user-home"
scripts/check.sh
```

Focused commands are:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./gradlew check --no-daemon
npm --prefix clients/vscode test
npm --prefix clients/vscode run validate
```

Keep generated outputs, local databases, `dist/`, and caches out of commits.

## Test-first contract

Start each behavior change with the smallest executable test that fails for the intended reason.
Add the behavior, keep the focused test green, then run the relevant broad gate.
A regression found by a broad gate receives a focused test before its fix.

Important contract areas are:

- JSON schema, protobuf round trips, IDs, exit codes, pagination, and response bounds.
- SQLite write-ahead log recovery, retention, impact widening, coverage, failures, and traces.
- `ASM` instrumentation, hostile values, redaction, Kotlin source maps, coroutines, TestNG, Gradle, and Maven.
- Zero-dependency API no-agent behavior, explicit captures, async context restoration, generated reports, ServiceLoader discovery, typed client argv, and publication metadata.
- Deterministic inputs, scheduling, fresh Java Virtual Machine (JVM) replay, reverse targets, and capability failures.
- Terminal user interface state/input/rendering and VS Code multi-root/trust/argv/size behavior.
- Cross-platform packages, checksums, `doctor`, and repository security policy.

Performance tests use warmed distributions, not a single timing.
Capability or stale failures are semantic results, never benchmark successes.

## CI and release checks

Pull requests and merge groups run Rust and JDK 21 checks on Linux, extension tests, repository lint, dependency checks, and secret and filesystem scans.
The `ci-success` gate requires every ordinary check to succeed, and branch protection requires the `required` summary.
Merges into `main` skip duplicate ordinary CI.

Release tags run Rust checks on all four supported operating-system targets, JVM checks on JDK 21 and 25 across those targets, and CodeQL analysis.
Package builds and smoke tests wait for those checks.
The extension passes its tests, manifest validation, and dependency audit before packaging.
Publication waits for both package jobs.
Scorecard runs weekly and when branch protection changes to check repository settings and external security signals.

## Changes and pull requests

- Keep public machine formats backward compatible within schema version `1`.
  Prefer additive nullable fields.
- Don't weaken bounds, redaction, conservative selection, or replay honesty.
- Use Conventional Commit subjects.
  The repository accepts squash merge only.
- Update documentation and fixtures with behavior changes.
- Confirm `scripts/check.sh` and applicable security checks before opening a pull request.
- Keep pull requests focused and complete the template.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
