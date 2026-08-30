#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "package-linux.sh requires Linux x86_64" >&2
  exit 2
fi

package_id="$(cargo pkgid -p walaru-cli)"
workspace_version="${package_id##*#}"
release_version="${1:-$workspace_version}"
if [[ "$release_version" != "$workspace_version" ]]; then
  echo "release version $release_version does not match workspace version $workspace_version" >&2
  exit 2
fi
archive_name="walaru-${release_version}-linux-x86_64"
temporary_root="$(mktemp -d)"
trap 'rm -rf "$temporary_root"' EXIT
bundle_root="$temporary_root/$archive_name"

export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$repository_root/.gradle-user-home}"
cargo build --release -p walaru-cli
./gradlew :jvm-api:jar :jvm-client:fatJar :jvm-testkit:jar :jvm-agent:fatJar :jvm-runner:fatJar :gradle-adapter:fatJar --no-daemon

install -Dm755 target/release/walaru "$bundle_root/bin/walaru"
install -Dm644 "jvm-agent/build/libs/jvm-agent-${workspace_version}-all.jar" "$bundle_root/lib/walaru-agent.jar"
install -Dm644 "jvm-api/build/libs/walaru-api-${workspace_version}.jar" "$bundle_root/lib/walaru-api.jar"
install -Dm644 "jvm-client/build/libs/walaru-client-${workspace_version}-all.jar" "$bundle_root/lib/walaru-client.jar"
install -Dm644 "jvm-testkit/build/libs/walaru-testkit-${workspace_version}.jar" "$bundle_root/lib/walaru-testkit.jar"
install -Dm644 "jvm-runner/build/libs/jvm-runner-${workspace_version}-all.jar" "$bundle_root/lib/walaru-runner.jar"
install -Dm644 "gradle-adapter/build/libs/gradle-adapter-${workspace_version}-all.jar" \
  "$bundle_root/lib/walaru-gradle-adapter.jar"
install -Dm644 gradle/walaru.init.gradle.kts "$bundle_root/share/walaru/walaru.init.gradle.kts"
install -Dm644 LICENSE-MIT "$bundle_root/LICENSE-MIT"
install -Dm644 LICENSE-APACHE "$bundle_root/LICENSE-APACHE"
install -Dm644 README.md "$bundle_root/README.md"
cp -R docs "$bundle_root/docs"
cp -R schemas "$bundle_root/schemas"
while IFS= read -r -d '' source; do
  mkdir -p "$(dirname "$bundle_root/$source")"
  install -m644 "$source" "$bundle_root/$source"
done < <(find examples -type f ! -path '*/build/*' ! -path '*/.gradle/*' -print0)
mkdir -p "$bundle_root/skills"
cp -R skills/walaru "$bundle_root/skills/walaru"
mkdir -p "$bundle_root/clients"
mkdir -p "$bundle_root/clients/vscode/media"
for source in package.json README.md CHANGELOG.md LICENSE .vscodeignore client.js extension.js model.js; do
  install -m644 "clients/vscode/$source" "$bundle_root/clients/vscode/$source"
done
install -m644 clients/vscode/media/walaru.svg "$bundle_root/clients/vscode/media/walaru.svg"
cp -R clients/intellij "$bundle_root/clients/intellij"

mkdir -p dist
tar -C "$temporary_root" -czf "dist/$archive_name.tar.gz" "$archive_name"
pushd dist >/dev/null
sha256sum "$archive_name.tar.gz" > "$archive_name.tar.gz.sha256"
popd >/dev/null
echo "dist/$archive_name.tar.gz"
