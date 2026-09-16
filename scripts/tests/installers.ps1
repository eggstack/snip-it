# Focused PowerShell installer contract suite (plain assertions, no Pester).
# Run on Windows CI: pwsh -NoProfile -File scripts/tests/installers.ps1
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$InstallerPath = Join-Path $ScriptDir '../../packaging/install.ps1'
# Dot-source loads helpers without running installation (guarded by
# $MyInvocation.InvocationName -ne '.' in install.ps1).
. $InstallerPath

$script:failures = 0

function Assert-Equal([string]$Expected, [string]$Actual, [string]$Label) {
    if ($Expected -ne $Actual) {
        Write-Host ("FAIL: " + $Label + ": expected '" + $Expected + "', got '" + $Actual + "'")
        $script:failures += 1
    }
}

function Assert-True([bool]$Condition, [string]$Label) {
    if (-not $Condition) {
        Write-Host ("FAIL: " + $Label + ": expected true")
        $script:failures += 1
    }
}

function Assert-False([bool]$Condition, [string]$Label) {
    if ($Condition) {
        Write-Host ("FAIL: " + $Label + ": expected false")
        $script:failures += 1
    }
}

function Expect-Throw([scriptblock]$Block, [string]$Needle, [string]$Label) {
    try {
        & $Block
        Write-Host ("FAIL: " + $Label + ": expected throw, succeeded")
        $script:failures += 1
    }
    catch {
        if ($Needle -and ($_.Exception.Message -notlike ("*" + $Needle + "*"))) {
            Write-Host ("FAIL: " + $Label + ": throw '" + $_.Exception.Message + "' did not contain '" + $Needle + "'")
            $script:failures += 1
        }
    }
}

# 1. Windows target mapping.
Assert-Equal 'x86_64-pc-windows-msvc' (Get-TargetForArchitecture 'X64') 'X64 mapping'
Assert-Equal 'aarch64-pc-windows-msvc' (Get-TargetForArchitecture 'Arm64') 'ARM64 mapping'
Assert-Equal 'source-only' (Get-TargetForArchitecture 'UnknownArch') 'unknown arch source-only'
$currentArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$currentTarget = Get-Target
if ($currentArch -eq 'X64') {
    Assert-Equal 'x86_64-pc-windows-msvc' $currentTarget 'current host X64 target'
}
elseif ($currentArch -eq 'Arm64') {
    Assert-Equal 'aarch64-pc-windows-msvc' $currentTarget 'current host ARM64 target'
}

# 2. Snp component mapping.
Assert-Equal 'snip-it' (Get-Package 'Snp') 'Snp package'
Assert-Equal 'snp' (Get-Binary 'Snp') 'Snp binary'
Assert-Equal 'v1.2.3' (Get-Tag 'Snp' '1.2.3') 'Snp tag'

# 3. Server component mapping.
Assert-Equal 'snip-sync' (Get-Package 'Server') 'Server package'
Assert-Equal 'snip-sync' (Get-Binary 'Server') 'Server binary'
Assert-Equal 'snip-sync-v0.1.4' (Get-Tag 'Server' '0.1.4') 'Server tag'

# 4. Asset names include .exe exactly once.
$snpAsset = Get-AssetName 'Snp' 'x86_64-pc-windows-msvc'
Assert-Equal 'snp-x86_64-pc-windows-msvc.exe' $snpAsset 'Snp asset name'
$serverAsset = Get-AssetName 'Server' 'x86_64-pc-windows-msvc'
Assert-Equal 'snip-sync-x86_64-pc-windows-msvc.exe' $serverAsset 'Server asset name'
foreach ($asset in @($snpAsset, $serverAsset)) {
    $exeCount = ([regex]::Matches($asset, '\.exe')).Count
    if ($exeCount -ne 1) {
        Write-Host "FAIL: asset '$asset' contains .exe $exeCount times, expected exactly once"
        $script:failures += 1
    }
}

# 5. SHA-256 sidecar parsing rejects malformed data.
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("snip-it-ps-test-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tempRoot | Out-Null
try {
    $candidate = Join-Path $tempRoot 'candidate.cmd'
    Set-Content -LiteralPath $candidate -Value "@echo off`r`necho snp 1.2.3`r`n" -Encoding Ascii
    $asset = 'snp-x86_64-pc-windows-msvc.exe'
    $goodHash = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
    $goodSidecar = Join-Path $tempRoot 'good.sha256'
    Set-Content -LiteralPath $goodSidecar -Value "$goodHash  $asset`r`n" -Encoding Ascii
    # Valid candidate passes.
    Assert-Candidate $candidate $goodSidecar $asset 'Snp' '1.2.3'

    $badSidecar = Join-Path $tempRoot 'bad.sha256'
    Set-Content -LiteralPath $badSidecar -Value "not-a-checksum`r`n" -Encoding Ascii
    Expect-Throw { Assert-Candidate $candidate $badSidecar $asset 'Snp' '1.2.3' } 'Malformed checksum' 'malformed sidecar rejected'

    $extraSidecar = Join-Path $tempRoot 'extra.sha256'
    Set-Content -LiteralPath $extraSidecar -Value "$goodHash  $asset extra`r`n" -Encoding Ascii
    Expect-Throw { Assert-Candidate $candidate $extraSidecar $asset 'Snp' '1.2.3' } 'Malformed checksum' 'extra field rejected'

    $wrongNameSidecar = Join-Path $tempRoot 'wrongname.sha256'
    Set-Content -LiteralPath $wrongNameSidecar -Value "$goodHash  wrong-name.exe`r`n" -Encoding Ascii
    Expect-Throw { Assert-Candidate $candidate $wrongNameSidecar $asset 'Snp' '1.2.3' } 'Malformed checksum' 'wrong asset name rejected'

    $mismatchSidecar = Join-Path $tempRoot 'mismatch.sha256'
    Set-Content -LiteralPath $mismatchSidecar -Value "0000000000000000000000000000000000000000000000000000000000000000  $asset`r`n" -Encoding Ascii
    Expect-Throw { Assert-Candidate $candidate $mismatchSidecar $asset 'Snp' '1.2.3' } 'SHA-256 mismatch' 'hash mismatch rejected'

    # 6. Candidate identity/version mismatch is rejected.
    $wrongId = Join-Path $tempRoot 'wrong-id.cmd'
    Set-Content -LiteralPath $wrongId -Value "@echo off`r`necho snip-sync 1.2.3`r`n" -Encoding Ascii
    $wrongIdHash = (Get-FileHash -LiteralPath $wrongId -Algorithm SHA256).Hash.ToLowerInvariant()
    $wrongIdSidecar = Join-Path $tempRoot 'wrong-id.sha256'
    Set-Content -LiteralPath $wrongIdSidecar -Value "$wrongIdHash  $asset`r`n" -Encoding Ascii
    Expect-Throw { Assert-Candidate $wrongId $wrongIdSidecar $asset 'Snp' '1.2.3' } 'does not match' 'wrong identity rejected'

    $wrongVer = Join-Path $tempRoot 'wrong-ver.cmd'
    Set-Content -LiteralPath $wrongVer -Value "@echo off`r`necho snp 0.0.0`r`n" -Encoding Ascii
    $wrongVerHash = (Get-FileHash -LiteralPath $wrongVer -Algorithm SHA256).Hash.ToLowerInvariant()
    $wrongVerSidecar = Join-Path $tempRoot 'wrong-ver.sha256'
    Set-Content -LiteralPath $wrongVerSidecar -Value "$wrongVerHash  $asset`r`n" -Encoding Ascii
    Expect-Throw { Assert-Candidate $wrongVer $wrongVerSidecar $asset 'Snp' '1.2.3' } 'does not match' 'wrong version rejected'
}
finally {
    if (Test-Path -LiteralPath $tempRoot) { Remove-Item -LiteralPath $tempRoot -Recurse -Force }
}

# 7. Both plus a single -Version remains rejected.
Expect-Throw { Assert-ComponentVersion 'Both' '1.2.3' } 'ambiguous' 'Both plus Version rejected'
Assert-ComponentVersion 'Snp' '1.2.3'
Assert-ComponentVersion 'Server' '0.9.9'
Assert-ComponentVersion 'Both' $null
Expect-Throw { Assert-ComponentVersion 'Snp' '1.2' } 'stable X.Y.Z' 'invalid version rejected'

# 8. User/admin destination logic remains deterministic.
$destination = Get-Destination
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
$isAdmin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($isAdmin) {
    Assert-Equal (Join-Path $env:ProgramFiles 'snip-it') $destination 'admin destination'
}
else {
    Assert-Equal (Join-Path $env:LOCALAPPDATA 'snip-it') $destination 'user destination'
}

# 9. Definite asset 404 permits fallback while other download failures do not.
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("snip-it-ps-fixture-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
$fixtureServer = Join-Path $fixtureRoot 'fixture_server.py'
Set-Content -LiteralPath $fixtureServer -Value @'
import http.server
import os
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/ok/payload.bin":
            body = b"payload"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path == "/fail500/asset":
            self.send_response(500)
            self.send_header("Content-Length", "5")
            self.end_headers()
            self.wfile.write(b"error")
            return
        self.send_response(404)
        self.send_header("Content-Length", "9")
        self.end_headers()
        self.wfile.write(b"not found")
    def log_message(self, *args):
        pass
http.server.ThreadingHTTPServer(("127.0.0.1", int(os.environ["FIXTURE_PORT"])), Handler).serve_forever()
'@ -Encoding Ascii
function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $port
}
$fixturePort = Get-FreePort
$env:FIXTURE_PORT = "$fixturePort"
$pythonCmd = 'python3'
if (-not (Get-Command $pythonCmd -ErrorAction SilentlyContinue)) {
    $pythonCmd = 'python'
}
$serverProcess = Start-Process -FilePath $pythonCmd -ArgumentList @($fixtureServer) -PassThru -WindowStyle Hidden
try {
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        try {
            Invoke-WebRequest -Uri "http://127.0.0.1:$fixturePort/ok/payload.bin" -UseBasicParsing -TimeoutSec 2 | Out-Null
            break
        }
        catch {
            Start-Sleep -Milliseconds 200
        }
    }
    $downloadDest = Join-Path $fixtureRoot 'out.bin'
    $ok = Invoke-Download "http://127.0.0.1:$fixturePort/ok/payload.bin" $downloadDest
    Assert-True $ok 'successful download returns true'
    # 404 with -AllowNotFound returns false (permits exact-version Cargo fallback).
    $notFound = Invoke-Download "http://127.0.0.1:$fixturePort/missing/asset" $downloadDest -AllowNotFound
    Assert-False $notFound 'asset 404 returns false'
    # 404 without -AllowNotFound is a hard failure (checksum path must not fall back).
    Expect-Throw { Invoke-Download "http://127.0.0.1:$fixturePort/missing/asset" $downloadDest } 'Download failed' '404 without AllowNotFound throws'
    # Non-404 server failure is a hard failure even with -AllowNotFound.
    Expect-Throw { Invoke-Download "http://127.0.0.1:$fixturePort/fail500/asset" $downloadDest -AllowNotFound } 'Download failed' '500 does not permit fallback'
}
finally {
    if ($serverProcess -and -not $serverProcess.HasExited) {
        Stop-Process -Id $serverProcess.Id -Force
        $serverProcess.WaitForExit()
    }
    Remove-Item Env:FIXTURE_PORT -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $fixtureRoot) { Remove-Item -LiteralPath $fixtureRoot -Recurse -Force }
}

# 10. ARM64 Windows remains source-only unless the release matrix adds it.
Assert-True (Test-SourceOnlyTarget 'aarch64-pc-windows-msvc') 'ARM64 Windows source-only'
Assert-True (Test-SourceOnlyTarget 'source-only') 'generic source-only'
Assert-False (Test-SourceOnlyTarget 'x86_64-pc-windows-msvc') 'x86_64 prebuilt is not source-only'

if ($script:failures -gt 0) {
    Write-Host "installer PowerShell contract tests failed: $($script:failures) failure(s)"
    exit 1
}
Write-Host 'installer PowerShell contract tests passed'
