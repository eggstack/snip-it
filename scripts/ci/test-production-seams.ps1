<#
.SYNOPSIS
    Production seam proof for Windows — verifies that a binary built without
    `test-support` cannot be influenced by test-only environment variables.

.DESCRIPTION
    This script:
    1. Builds `snp` without `test-support` into an isolated target directory.
    2. Sets matching valid seam values and runs valid scenarios.
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

try {
    Write-Host ''
    Write-Host '=== Test 1: SNP_TEST_FAILPOINT does not abort production binary ==='
    $env:SNP_TEST_FAILPOINT = 'restore-after-prepared'
    & $Binary list *>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'FAIL: production binary aborted or errored with matching failpoint'
        exit 1
    }
    Write-Host 'PASS: failpoint did not abort production binary'

    Write-Host ''
    Write-Host '=== Test 2: SNP_TEST_EXECUTOR_MODE does not bypass executor ==='
    $env:SNP_TEST_EXECUTOR_MODE = 'noop-success'
    $StateDir = Join-Path $TmpDir 'state'
    & $Binary auto-sync-execute --state-dir $StateDir *>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Host 'FAIL: production executor exited 0 with noop-success mode (seam is active)'
        exit 1
    }
    Write-Host 'PASS: noop-success mode did not bypass production executor'

    Write-Host ''
    Write-Host '=== Test 3: SNP_SKIP_WORKER_SPAWN does not suppress production scheduling ==='
    $env:SNP_SKIP_WORKER_SPAWN = '1'
    & $Binary list *>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'FAIL: production binary errored with SNP_SKIP_WORKER_SPAWN set'
        exit 1
    }
    Write-Host 'PASS: worker spawn suppression did not affect production binary'

    Write-Host ''
    Write-Host '=== Test 4: SNP_TEST_EVENTS_DIR does not create event files ==='
    $EventsDir = Join-Path $TmpDir 'events'
    New-Item -ItemType Directory -Path $EventsDir -Force | Out-Null
    $env:SNP_TEST_EVENTS_DIR = $EventsDir
    & $Binary list *>$null
    if (Test-Path (Join-Path $EventsDir 'test-events.jsonl')) {
        Write-Host 'FAIL: production binary created event file'
        exit 1
    }
    Write-Host 'PASS: no event file created in production binary'

    Write-Host ''
    Write-Host '=== Test 5: SNP_TEST_MUTATION_BARRIER_DIR does not block production ==='
    $BarrierDir = Join-Path $TmpDir 'barrier'
    New-Item -ItemType Directory -Path $BarrierDir -Force | Out-Null
    'snippet-save' | Set-Content (Join-Path $BarrierDir 'point')
    $env:SNP_TEST_MUTATION_BARRIER_DIR = $BarrierDir
    & $Binary list *>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'FAIL: production binary blocked or errored with mutation barrier set'
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
