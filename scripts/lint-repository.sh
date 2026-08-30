#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

if rg -n --pcre2 'uses:\s*[^@\s]+@(?![0-9a-f]{40}(?:\s|\s*#|$))' .github/workflows; then
  echo "all GitHub Actions must use a full commit SHA" >&2
  exit 1
fi

shellcheck scripts/*.sh
actionlint
zizmor --offline --pedantic .
node clients/vscode/scripts/validate-manifest.mjs

for document in .github/rulesets/*.json .github/repository-settings.json clients/vscode/package.json schemas/*.json; do
  node -e 'JSON.parse(require("node:fs").readFileSync(process.argv[1], "utf8"))' "$document"
done
