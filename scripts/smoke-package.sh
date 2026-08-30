#!/usr/bin/env bash
set -euo pipefail

archive="${1:?usage: smoke-package.sh ARCHIVE}"
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
for library in walaru-api.jar walaru-client.jar walaru-testkit.jar walaru-agent.jar; do
  if [[ ! -f "$bundle_root/lib/$library" ]]; then
    echo "package is missing lib/$library" >&2
    exit 2
  fi
done
if ! jar tf "$bundle_root/lib/walaru-api.jar" | grep -F 'io/github/p4suta/walaru/Walaru.class' >/dev/null; then
  echo "walaru-api.jar does not contain the public API" >&2
  exit 2
fi
if ! jar tf "$bundle_root/lib/walaru-client.jar" | grep -F 'io/github/p4suta/walaru/client/WalaruClient.class' >/dev/null; then
  echo "walaru-client.jar does not contain the typed client" >&2
  exit 2
fi

smoke_workspace="$temporary_root/workspace"
mkdir -p "$smoke_workspace"
printf 'rootProject.name = "walaru-package-smoke"\n' > "$smoke_workspace/settings.gradle.kts"
: > "$smoke_workspace/gradlew"

doctor_output="$temporary_root/doctor.json"
if ! "$binary" --workspace "$smoke_workspace" --format json doctor > "$doctor_output"; then
  find "$smoke_workspace/.gradle/walaru" -name daemon.log -type f \
    -exec sh -c 'echo "daemon log: $1"; tail -n 100 "$1"' sh {} \; 2>/dev/null || true
  exit 3
fi
node -e '
const fs = require("node:fs");
const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
if (value.schemaVersion !== "1" || value.data?.ready !== true) process.exit(1);
' "$doctor_output"
"$binary" --workspace "$smoke_workspace" --format json stop >/dev/null
