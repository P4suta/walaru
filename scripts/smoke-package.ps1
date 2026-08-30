$ErrorActionPreference = "Stop"

function Invoke-Walaru {
    param(
        [Parameter(Mandatory = $true)][string]$Binary,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [int]$TimeoutSeconds = 60
    )

    $StartInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $StartInfo.FileName = $Binary
    $StartInfo.UseShellExecute = $false
    $StartInfo.RedirectStandardOutput = $true
    $StartInfo.RedirectStandardError = $true
    foreach ($Argument in $Arguments) {
        [void]$StartInfo.ArgumentList.Add($Argument)
    }
    $Process = [System.Diagnostics.Process]::new()
    $Process.StartInfo = $StartInfo
    [void]$Process.Start()
    # Walaru's structured response is exactly one JSON line. Reading to EOF can
    # wait forever on Windows if the background daemon retains an inherited
    # pipe handle after the short-lived client exits.
    $StandardOutput = $Process.StandardOutput.ReadLineAsync()
    $StandardError = $Process.StandardError.ReadLineAsync()
    if (-not $Process.WaitForExit($TimeoutSeconds * 1000)) {
        $Process.Kill($true)
        $Process.WaitForExit()
        throw "walaru timed out after $TimeoutSeconds seconds: $($Arguments -join ' ')"
    }
    if (-not $StandardOutput.Wait(5000)) {
        throw "walaru exited without completing its structured output: $($Arguments -join ' ')"
    }
    $ErrorText = ""
    if ($StandardError.Wait(250)) {
        $ErrorText = $StandardError.GetAwaiter().GetResult()
    }
    [pscustomobject]@{
        ExitCode = $Process.ExitCode
        StandardOutput = $StandardOutput.GetAwaiter().GetResult()
        StandardError = $ErrorText
    }
}

function Write-DaemonLogs {
    param([Parameter(Mandatory = $true)][string]$WorkspaceRoot)

    $StateRoot = Join-Path $WorkspaceRoot ".gradle\walaru"
    if (-not (Test-Path $StateRoot)) { return }
    Get-ChildItem -Path $StateRoot -Filter daemon.log -Recurse -ErrorAction SilentlyContinue |
        ForEach-Object {
            Write-Host "daemon log: $($_.FullName)"
            Get-Content $_.FullName -ErrorAction SilentlyContinue
        }
}

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
    $SmokeWorkspace = Join-Path $TemporaryRoot "workspace"
    New-Item -ItemType Directory -Force -Path $SmokeWorkspace | Out-Null
    'rootProject.name = "walaru-package-smoke"' |
        Set-Content -Encoding utf8 (Join-Path $SmokeWorkspace "settings.gradle.kts")
    New-Item -ItemType File -Force -Path (Join-Path $SmokeWorkspace "gradlew.bat") | Out-Null
    $DoctorResult = Invoke-Walaru -Binary $Binary -Arguments @(
        "--workspace", $SmokeWorkspace, "--format", "json", "doctor"
    ) -TimeoutSeconds 90
    if ($DoctorResult.ExitCode -ne 0) {
        Write-Host $DoctorResult.StandardError
        throw "packaged doctor smoke test failed"
    }
    $Doctor = $DoctorResult.StandardOutput | ConvertFrom-Json
    if ($Doctor.schemaVersion -ne "1" -or $Doctor.data.ready -ne $true) {
        throw "packaged doctor returned an invalid or unready response"
    }
    $StopResult = Invoke-Walaru -Binary $Binary -Arguments @(
        "--workspace", $SmokeWorkspace, "--format", "json", "stop"
    ) -TimeoutSeconds 30
    if ($StopResult.ExitCode -ne 0) {
        Write-Host $StopResult.StandardError
        throw "packaged daemon did not stop cleanly"
    }
} catch {
    if ($SmokeWorkspace) { Write-DaemonLogs -WorkspaceRoot $SmokeWorkspace }
    throw
} finally {
    if (Test-Path $TemporaryRoot) { Remove-Item -Recurse -Force $TemporaryRoot }
}
