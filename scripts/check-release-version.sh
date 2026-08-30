#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

tag="${1:?usage: check-release-version.sh vVERSION}"
package_id="$(cargo pkgid -p walaru-cli)"
cargo_version="${package_id##*#}"
tag_version="${tag#v}"
vscode_version="$(node -p 'require("./clients/vscode/package.json").version')"

if [[ "$tag" != v* || "$tag_version" != "$cargo_version" || "$vscode_version" != "$cargo_version" ]]; then
  echo "tag ($tag), Cargo ($cargo_version), and VS Code ($vscode_version) versions must match" >&2
  exit 2
fi
