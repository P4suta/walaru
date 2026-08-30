#!/usr/bin/env bash
set -euo pipefail

archive="${1:?usage: smoke-package.sh ARCHIVE}"
repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")"
checksum="$archive.sha256"

if [[ ! -f "$archive" || ! -f "$checksum" ]]; then
  echo "archive and checksum are required" >&2
  exit 2
fi

archive_directory="$(dirname "$archive")"
archive_name="$(basename "$archive")"
pushd "$archive_directory" >/dev/null
if [[ "$(uname -s)" == "Darwin" ]]; then
  shasum -a 256 -c "$(basename "$checksum")"
else
  sha256sum -c "$(basename "$checksum")"
fi
popd >/dev/null

temporary_root="$(mktemp -d)"
trap 'rm -rf "$temporary_root"' EXIT
tar -xzf "$archive" -C "$temporary_root"
bundle_root="$temporary_root/${archive_name%.tar.gz}"
binary="$bundle_root/bin/walaru"
if [[ ! -x "$binary" ]]; then
  echo "package is missing bin/walaru" >&2
  exit 2
fi

doctor_output="$temporary_root/doctor.json"
"$binary" --workspace "$repository_root" --format json doctor > "$doctor_output"
node -e '
const fs = require("node:fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
if (value.schemaVersion !== "1" || value.data?.ready !== true) process.exit(1);
' "$doctor_output"
"$binary" --workspace "$repository_root" --format json stop >/dev/null
