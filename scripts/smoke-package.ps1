$ErrorActionPreference = "Stop"

$Archive = $args[0]
if (-not $Archive) { throw "usage: smoke-package.ps1 ARCHIVE" }
$Archive = (Resolve-Path $Archive).Path
$Checksum = "$Archive.sha256"
if (-not (Test-Path $Checksum)) { throw "missing $Checksum" }

$Expected = ((Get-Content $Checksum -Raw) -split '\s+')[0].ToLowerInvariant()
$Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
if ($Expected -ne $Actual) { throw "checksum mismatch" }

$TemporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
try {
    Expand-Archive -Path $Archive -DestinationPath $TemporaryRoot
    $BundleName = [System.IO.Path]::GetFileNameWithoutExtension($Archive)
    $Binary = Join-Path $TemporaryRoot "$BundleName\bin\walaru.exe"
    if (-not (Test-Path $Binary)) { throw "package is missing bin\walaru.exe" }
    $RepositoryRoot = Split-Path -Parent $PSScriptRoot
    $Doctor = & $Binary --workspace $RepositoryRoot --format json doctor | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $Doctor.schemaVersion -ne "1" -or $Doctor.data.ready -ne $true) {
        throw "packaged doctor smoke test failed"
    }
    & $Binary --workspace $RepositoryRoot --format json stop | Out-Null
} finally {
    if (Test-Path $TemporaryRoot) { Remove-Item -Recurse -Force $TemporaryRoot }
}
