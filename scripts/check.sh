#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node --test clients/vscode/test/*.test.js
node clients/vscode/scripts/validate-manifest.mjs

export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$repository_root/.gradle-user-home}"
./gradlew check --no-daemon
