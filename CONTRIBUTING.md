# Contributing to Walaru

Thank you for helping build Walaru. Public communication, code, tests, and documentation are in English.

## Development setup

Install Rust 1.97, JDK 21+, Node.js 22+, and a POSIX shell on Linux/macOS (PowerShell is supported for Windows packaging). Then run:

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

Generated outputs, local databases, `dist/`, and caches stay in place locally but must not be committed.

## Test-first contract

Start each behavior change with the smallest executable test that fails for the intended reason. Implement the behavior, keep the focused test green, then run the relevant broad gate. A regression found by a broad gate receives a focused test before its fix.

Important contract areas are:

- JSON schema, protobuf round trips, IDs, exit codes, pagination, and response bounds.
- SQLite WAL recovery, retention, impact widening, coverage, failures, and traces.
- ASM instrumentation, hostile values, redaction, Kotlin source maps, coroutines, TestNG, Gradle, and Maven.
- Zero-dependency API no-agent behavior, explicit captures, async context restoration, generated
  reports, ServiceLoader discovery, typed client argv, and publication metadata.
- Deterministic inputs, scheduling, fresh-JVM replay, reverse targets, and capability failures.
- TUI state/input/rendering and VS Code multi-root/trust/argv/size behavior.
- Cross-platform packages, checksums, `doctor`, and repository security policy.

Performance tests use warmed distributions, not a single timing. Capability or stale failures are semantic results, never benchmark successes.

## Changes and pull requests

- Keep public machine formats backward compatible within schema version `1`; additive nullable fields are preferred.
- Do not weaken bounds, redaction, conservative selection, or replay honesty.
- Use Conventional Commit subjects. The repository accepts squash merge only.
- Update documentation and fixtures with behavior changes.
- Confirm `scripts/check.sh` and applicable security checks before opening a pull request.
- Keep pull requests focused and complete the template.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
