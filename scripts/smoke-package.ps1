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
    try {
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
    } finally {
        $Process.Dispose()
    }
}

function Wait-WalaruDaemonExit {
    param([Parameter(Mandatory = $true)][string]$WorkspaceRoot)

    $StateRoot = Join-Path $WorkspaceRoot ".gradle\walaru"
    $Deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $Deadline) {
        $Metadata = Get-ChildItem -Path $StateRoot -Filter daemon.json -Recurse -ErrorAction SilentlyContinue
        if (-not $Metadata) { return }
        Start-Sleep -Milliseconds 100
    }
    throw "packaged daemon did not exit within 10 seconds"
}

function Remove-TemporaryDirectory {
    param([Parameter(Mandatory = $true)][string]$Path)

    foreach ($Attempt in 1..20) {
        try {
            Remove-Item -Recurse -Force $Path -ErrorAction Stop
            return
        } catch {
            if ($Attempt -eq 20) { throw }
            Start-Sleep -Milliseconds 250
        }
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
    foreach ($Library in "walaru-api.jar", "walaru-client.jar", "walaru-testkit.jar", "walaru-agent.jar") {
        if (-not (Test-Path (Join-Path $TemporaryRoot "$BundleName\lib\$Library"))) {
            throw "package is missing lib\$Library"
        }
    }
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
    Wait-WalaruDaemonExit -WorkspaceRoot $SmokeWorkspace
} catch {
    if ($SmokeWorkspace) { Write-DaemonLogs -WorkspaceRoot $SmokeWorkspace }
    throw
} finally {
    if (Test-Path $TemporaryRoot) { Remove-TemporaryDirectory -Path $TemporaryRoot }
}
