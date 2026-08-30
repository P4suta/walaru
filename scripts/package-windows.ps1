$ErrorActionPreference = "Stop"

$RepositoryRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepositoryRoot
$PackageId = cargo pkgid -p walaru-cli
$WorkspaceVersion = ($PackageId -split '#')[-1]
$ReleaseVersion = if ($args.Count -gt 0) { $args[0] } else { $WorkspaceVersion }
if ($ReleaseVersion -ne $WorkspaceVersion) {
    throw "release version $ReleaseVersion does not match workspace version $WorkspaceVersion"
}
$ArchiveName = "walaru-$ReleaseVersion-windows-x86_64"
$TemporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
$BundleRoot = Join-Path $TemporaryRoot $ArchiveName

try {
    cargo build --release -p walaru-cli
    & .\gradlew.bat :jvm-api:jar :jvm-client:fatJar :jvm-testkit:jar :jvm-agent:fatJar :jvm-runner:fatJar :gradle-adapter:fatJar --no-daemon
    New-Item -ItemType Directory -Force -Path "$BundleRoot\bin", "$BundleRoot\lib", "$BundleRoot\share\walaru", "$BundleRoot\skills", "$BundleRoot\clients\vscode\media" | Out-Null
    Copy-Item target\release\walaru.exe "$BundleRoot\bin\walaru.exe"
    Copy-Item "jvm-agent\build\libs\jvm-agent-$WorkspaceVersion-all.jar" "$BundleRoot\lib\walaru-agent.jar"
    Copy-Item "jvm-api\build\libs\walaru-api-$WorkspaceVersion.jar" "$BundleRoot\lib\walaru-api.jar"
    Copy-Item "jvm-client\build\libs\walaru-client-$WorkspaceVersion-all.jar" "$BundleRoot\lib\walaru-client.jar"
    Copy-Item "jvm-testkit\build\libs\walaru-testkit-$WorkspaceVersion.jar" "$BundleRoot\lib\walaru-testkit.jar"
    Copy-Item "jvm-runner\build\libs\jvm-runner-$WorkspaceVersion-all.jar" "$BundleRoot\lib\walaru-runner.jar"
    Copy-Item "gradle-adapter\build\libs\gradle-adapter-$WorkspaceVersion-all.jar" "$BundleRoot\lib\walaru-gradle-adapter.jar"
    Copy-Item gradle\walaru.init.gradle.kts "$BundleRoot\share\walaru\walaru.init.gradle.kts"
    Copy-Item LICENSE-MIT, LICENSE-APACHE $BundleRoot
    Copy-Item README.md "$BundleRoot\README.md"
    Copy-Item -Recurse docs "$BundleRoot\docs"
    Copy-Item -Recurse schemas "$BundleRoot\schemas"
    Get-ChildItem examples -File -Recurse |
        Where-Object { $_.FullName -notmatch '[\\/]build[\\/]|[\\/]\.gradle[\\/]' } |
        ForEach-Object {
            $Relative = [System.IO.Path]::GetRelativePath($RepositoryRoot, $_.FullName)
            $Target = Join-Path $BundleRoot $Relative
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Target) | Out-Null
            Copy-Item $_.FullName $Target
        }
    Copy-Item -Recurse skills\walaru "$BundleRoot\skills\walaru"
    $VsCodeFiles = "package.json", "README.md", "CHANGELOG.md", "LICENSE", ".vscodeignore", "client.js", "extension.js", "model.js"
    foreach ($File in $VsCodeFiles) {
        Copy-Item "clients\vscode\$File" "$BundleRoot\clients\vscode\$File"
    }
    Copy-Item clients\vscode\media\walaru.svg "$BundleRoot\clients\vscode\media\walaru.svg"
    Copy-Item -Recurse clients\intellij "$BundleRoot\clients\intellij"
    New-Item -ItemType Directory -Force -Path dist | Out-Null
    $Archive = "dist\$ArchiveName.zip"
    Compress-Archive -Path $BundleRoot -DestinationPath $Archive -Force
    $Hash = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    "$Hash  $ArchiveName.zip" | Set-Content -Encoding ascii "$Archive.sha256"
    Write-Output $Archive
} finally {
    if (Test-Path $TemporaryRoot) { Remove-Item -Recurse -Force $TemporaryRoot }
}
