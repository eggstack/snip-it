<#
.SYNOPSIS
    Production seam proof for Windows — verifies that a binary built without
    `test-support` cannot be influenced by test-only environment variables.

.DESCRIPTION
    This script:
    1. Builds `snp` without `test-support` into an isolated target directory.
    2. Sets matching valid seam values and runs valid scenarios that traverse
      the guarded code paths.
    3. Asserts that no test behavior activates.
#>
param()

$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path (Join-Path $ScriptDir '..\..')
Set-Location $RepoRoot

$TargetDir = 'target\production-seam'
$Binary = Join-Path $TargetDir 'release\snp.exe'

Write-Host '=== Building production binary (no test-support) ==='
cargo build --release --no-default-features --target-dir $TargetDir

if (-not (Test-Path $Binary)) {
    Write-Host "FAIL: production binary not found at $Binary"
    exit 1
}

# Create a temporary config dir for the test scenario.
$TmpDir = Join-Path $env:TEMP ('snp-seam-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null
$ConfigHome = Join-Path $TmpDir 'config'
$SnipConfig = Join-Path $ConfigHome 'snp'
New-Item -ItemType Directory -Path $SnipConfig -Force | Out-Null

$env:XDG_CONFIG_HOME = $ConfigHome
$env:SNP_ALLOW_PLAINTEXT_API_KEY = 'true'

# Write a minimal sync.toml
@'
[settings.sync]
enabled = true
server_url = "http://127.0.0.1:1"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = false
'@ | Set-Content (Join-Path $SnipConfig 'sync.toml')

# Helper: bounded wait for a file to appear
function Wait-ForFile {
    param([string]$Path, [int]$TimeoutSecs = 10)
    $elapsed = 0
    while (-not (Test-Path $Path)) {
        Start-Sleep -Milliseconds 200
        $elapsed++
        if ($elapsed -ge ($TimeoutSecs * 5)) { return $false }
    }
    return $true
}

try {
    Write-Host ''
    Write-Host '=== Test 1: SNP_TEST_FAILPOINT does not abort production restore ==='
    # Create a valid backup to restore.
    $BackupDir = Join-Path $TmpDir 'valid-backup'
    $LibDir = Join-Path $BackupDir 'libraries'
    New-Item -ItemType Directory -Path $LibDir -Force | Out-Null

    $LibContent = '[[snippets]]
id = "test-1"
description = "test snippet"
command = "echo test"
'
    $LibContent | Set-Content (Join-Path $LibDir 'default.toml')

    $IndexContent = '[[libraries]]
filename = "default"
is_primary = true
'
    $IndexContent | Set-Content (Join-Path $BackupDir 'libraries.toml')

    $LibSha = (Get-FileHash -Path (Join-Path $LibDir 'default.toml') -Algorithm SHA256).Hash.ToLower()
    $IndexSha = (Get-FileHash -Path (Join-Path $BackupDir 'libraries.toml') -Algorithm SHA256).Hash.ToLower()
    $LibSize = (Get-Item (Join-Path $LibDir 'default.toml')).Length
    $IndexSize = (Get-Item (Join-Path $BackupDir 'libraries.toml')).Length

    $Manifest = @"
schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = $($LibSize)
sha256 = "$LibSha"

[[files]]
path = "libraries.toml"
kind = "index"
size = $($IndexSize)
sha256 = "$IndexSha"
"@
    $Manifest | Set-Content (Join-Path $BackupDir 'manifest.toml')

    # Run restore with the failpoint env var set. Production binary ignores it.
    $env:SNP_TEST_FAILPOINT = 'restore-after-prepared'
    & $Binary restore $BackupDir --mode dry-run *>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'FAIL: production binary aborted or errored with matching failpoint'
        exit 1
    }
    Write-Host 'PASS: failpoint did not abort production restore'

    Write-Host ''
    Write-Host '=== Test 2: SNP_SKIP_WORKER_SPAWN does not suppress production scheduling ==='
    $env:SNP_SKIP_WORKER_SPAWN = '1'
    & $Binary library create seam-test *>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'FAIL: production binary errored with SNP_SKIP_WORKER_SPAWN set during real mutation'
        exit 1
    }
    $libPath = Join-Path $SnipConfig 'libraries\seam-test.toml'
    if (-not (Test-Path $libPath)) {
        Write-Host 'FAIL: library file was not created — mutation was suppressed'
        exit 1
    }
    Write-Host 'PASS: worker spawn suppression did not affect production mutation'

    Write-Host ''
    Write-Host '=== Test 4: SNP_TEST_EVENTS_DIR does not create event files ==='
    $EventsDir = Join-Path $TmpDir 'events'
    New-Item -ItemType Directory -Path $EventsDir -Force | Out-Null
    $env:SNP_TEST_EVENTS_DIR = $EventsDir
    & $Binary auto-sync-worker --state-dir $StateDir *>$null
    $null = $LASTEXITCODE  # may fail due to unreachable server; that's expected
    if (Test-Path (Join-Path $EventsDir 'test-events.jsonl')) {
        Write-Host 'FAIL: production binary created event file'
        exit 1
    }
    Write-Host 'PASS: no event file created in production binary'

    Write-Host ''
    Write-Host '=== Test 5: SNP_TEST_MUTATION_BARRIER_DIR does not block production ==='
    $BarrierDir = Join-Path $TmpDir 'barrier'
    New-Item -ItemType Directory -Path $BarrierDir -Force | Out-Null
    'library-create' | Set-Content (Join-Path $BarrierDir 'point')
    $env:SNP_TEST_MUTATION_BARRIER_DIR = $BarrierDir
    $job = Start-Job { & $using:Binary library create barrier-test *>$null; $LASTEXITCODE }
    $completed = $job | Wait-Job -Timeout 10
    if (-not $completed) {
        Write-Host 'FAIL: production binary blocked with mutation barrier set (timeout)'
        Stop-Job $job
        exit 1
    }
    $exitCode5 = (Receive-Job $job)
    Remove-Job $job
    if ($exitCode5 -ne 0) {
        Write-Host "FAIL: production binary errored with mutation barrier set (exit: $exitCode5)"
        exit 1
    }
    if (Test-Path (Join-Path $BarrierDir 'entered')) {
        Write-Host 'FAIL: production binary entered mutation barrier'
        exit 1
    }
    Write-Host 'PASS: mutation barrier did not block production binary'

    Write-Host ''
    Write-Host '=== All production seam tests passed ==='
}
finally {
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}
