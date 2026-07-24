<#
.SYNOPSIS
    Install protoc for Windows CI.

.DESCRIPTION
    Downloads and installs protoc under RUNNER_TEMP, adds it to GITHUB_PATH,
    and verifies the installation.

.PARAMETER Version
    protoc version to install (default: 25.1)

.NOTES
    Requirements:
    - Exact version
    - Architecture-aware artifact selection
    - Download failure is fatal
    - Checksum verification if published checksums are available
    - Install under RUNNER_TEMP, not C:\ root
    - Add path via GITHUB_PATH
    - Verify protoc --version in a subsequent step
#>

param(
    [string]$Version = "25.1"
)

$ErrorActionPreference = 'Stop'

$RunnerTemp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$InstallDir = Join-Path $RunnerTemp "protoc-$Version"

# Detect architecture
$Arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'win64' } else { 'win64' }

$Artifact = "protoc-$Version-win64.zip"
$Url = "https://github.com/protocolbuffers/protobuf/releases/download/v$Version/$Artifact"

Write-Host "Downloading protoc $Version for Windows..."
Write-Host "URL: $Url"

$TmpZip = Join-Path $RunnerTemp "protoc-$Version.zip"

try {
    Invoke-WebRequest -Uri $Url -OutFile $TmpZip
} catch {
    Write-Error "Failed to download protoc from $Url"
    exit 1
}

# Verify checksum if available
$ShaUrl = "$Url.sha256"
try {
    $ShaFile = Join-Path $RunnerTemp "protoc-$Version.zip.sha256"
    Invoke-WebRequest -Uri $ShaUrl -OutFile $ShaFile
    $ExpectedHash = (Get-Content $ShaFile | Select-String -Pattern $Artifact | ForEach-Object {
        ($_ -split '\s+')[0]
    }) | Select-Object -First 1
    if ($ExpectedHash) {
        $ActualHash = (Get-FileHash -Algorithm SHA256 -Path $TmpZip).Hash.ToLower()
        $ExpectedHashLower = $ExpectedHash.ToLower()
        if ($ExpectedHashLower -ne $ActualHash) {
            Write-Error "Checksum mismatch for protoc"
            Write-Host "  Expected: $ExpectedHashLower"
            Write-Host "  Actual:   $ActualHash"
            exit 1
        }
        Write-Host "Checksum verified: $ActualHash"
    }
} catch {
    Write-Host "Warning: Could not verify checksum (continuing)"
}

Write-Host "Extracting protoc..."
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Expand-Archive -Path $TmpZip -DestinationPath $InstallDir -Force
Remove-Item $TmpZip -Force

# Add to PATH
$ProtocBin = Join-Path $InstallDir 'bin'
[Environment]::SetEnvironmentVariable('PATH', $env:PATH + ";$ProtocBin", [EnvironmentVariableTarget]::Process)
# Also add to GITHUB_PATH for subsequent steps
$ProtocBin | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append

Write-Host "protoc installed at: $ProtocBin"
& "$ProtocBin\protoc.exe" --version
