# Install released snip-it binaries, falling back to an exact-version Cargo
# build only when the selected host has no published binary asset.
[CmdletBinding()]
param(
    [ValidateSet('Snp', 'Server', 'Both')]
    [string]$Component = 'Snp',
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$GitHubBaseDefault = 'https://github.com/eggstack/snip-it/releases/download'
$CratesBaseDefault = 'https://crates.io/api/v1'

function Test-InstallerMode {
    return $env:SNP_INSTALL_TEST_MODE -eq '1'
}

function Get-GitHubBase {
    if ((Test-InstallerMode) -and $env:SNP_INSTALL_GITHUB_BASE) {
        return $env:SNP_INSTALL_GITHUB_BASE.TrimEnd('/')
    }
    return $GitHubBaseDefault
}

function Get-CratesBase {
    if ((Test-InstallerMode) -and $env:SNP_INSTALL_CRATES_API_BASE) {
        return $env:SNP_INSTALL_CRATES_API_BASE.TrimEnd('/')
    }
    return $CratesBaseDefault
}

function Get-Target {
    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($architecture.ToString()) {
        'X64' { return 'x86_64-pc-windows-msvc' }
        'Arm64' { return 'aarch64-pc-windows-msvc' }
        default { return 'source-only' }
    }
}

function Get-Package([string]$Name) {
    if ($Name -eq 'Snp') { return 'snip-it' }
    if ($Name -eq 'Server') { return 'snip-sync' }
    throw "Unknown component '$Name'"
}

function Get-Binary([string]$Name) {
    if ($Name -eq 'Snp') { return 'snp' }
    if ($Name -eq 'Server') { return 'snip-sync' }
    throw "Unknown component '$Name'"
}

function Get-Tag([string]$Name, [string]$SelectedVersion) {
    if ($Name -eq 'Snp') { return "v$SelectedVersion" }
    if ($Name -eq 'Server') { return "snip-sync-v$SelectedVersion" }
    throw "Unknown component '$Name'"
}

function Test-StableVersion([string]$SelectedVersion) {
    if ($SelectedVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') {
        throw "Version '$SelectedVersion' is not a stable X.Y.Z version"
    }
}

function Invoke-Download([string]$Uri, [string]$Destination, [switch]$AllowNotFound) {
    try {
        Invoke-WebRequest -Uri $Uri -OutFile $Destination -UseBasicParsing
        return $true
    }
    catch {
        $status = $null
        if ($_.Exception.Response) {
            $status = [int]$_.Exception.Response.StatusCode
        }
        if ($AllowNotFound -and $status -eq 404) {
            return $false
        }
        throw "Download failed with HTTP ${status}: $Uri"
    }
}

function Get-CrateVersion([string]$Package) {
    $metadata = Invoke-RestMethod -Uri "$(Get-CratesBase)/crates/$Package" -Method Get
    $SelectedVersion = $metadata.crate.max_stable_version
    if (-not $SelectedVersion) { $SelectedVersion = $metadata.crate.max_version }
    Test-StableVersion $SelectedVersion
    return $SelectedVersion
}

function Assert-Candidate([string]$Candidate, [string]$Checksum, [string]$Asset,
    [string]$Name, [string]$SelectedVersion) {
    $lines = @(Get-Content -LiteralPath $Checksum | Where-Object { $_.Trim() -ne '' })
    if ($lines.Count -ne 1) { throw "Malformed checksum sidecar for $Asset" }
    $parts = $lines[0].Trim() -split '\s+'
    if ($parts.Count -ne 2 -or $parts[0] -notmatch '^[0-9a-fA-F]{64}$' -or $parts[1] -ne $Asset) {
        throw "Malformed checksum sidecar for $Asset"
    }
    $actual = (Get-FileHash -LiteralPath $Candidate -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $parts[0].ToLowerInvariant()) { throw "SHA-256 mismatch for $Asset" }

    Assert-BinaryIdentity $Candidate $Asset $Name $SelectedVersion
}

function Assert-BinaryIdentity([string]$Candidate, [string]$Asset,
    [string]$Name, [string]$SelectedVersion) {
    $identity = (& $Candidate version 2>&1 | Out-String).Trim()
    $expected = "$(Get-Binary $Name) $SelectedVersion"
    if ($LASTEXITCODE -ne 0 -or $identity -ne $expected) {
        throw "Candidate identity '$identity' does not match '$expected'"
    }
}

function Get-CargoCandidate([string]$Name, [string]$SelectedVersion, [string]$Root, [string]$Target) {
    $Package = Get-Package $Name
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "No prebuilt binary exists for target '$Target'. Install Rust/Cargo and run: cargo install $Package --version '=$SelectedVersion' --locked"
    }
    & cargo install $Package --version "=$SelectedVersion" --locked --root $Root
    if ($LASTEXITCODE -ne 0) { throw "Cargo failed to build $Package $SelectedVersion" }
    $Candidate = Join-Path $Root "bin\$(Get-Binary $Name).exe"
    if (-not (Test-Path -LiteralPath $Candidate)) { throw "Cargo did not produce $Candidate" }
    Assert-BinaryIdentity $Candidate $Binary $Name $SelectedVersion
    return $Candidate
}

function Get-Destination {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    $isAdmin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if ($isAdmin) { return (Join-Path $env:ProgramFiles 'snip-it') }
    return (Join-Path $env:LOCALAPPDATA 'snip-it')
}

function Test-PathEntry([string]$Directory) {
    return (($env:Path -split ';') -contains $Directory)
}

function Install-ServerSetup([string]$ServerBinary) {
    try {
        & $ServerBinary init --skip-cert
        if ($LASTEXITCODE -ne 0) { throw 'init failed' }
    }
    catch {
        Write-Warning "snip-sync binary installed, but layout initialization needs attention: $($_.Exception.Message)"
        return
    }

    $help = (& $ServerBinary --help 2>&1 | Out-String)
    if ($help -match '(^|\s)startup(\s|$)') {
        & $ServerBinary startup install
        if ($LASTEXITCODE -eq 0) {
            Write-Host 'snip-sync startup registration completed.'
            return
        }
        Write-Warning 'snip-sync installed, but startup registration was not completed.'
    }
    else {
        Write-Host 'snip-sync installed and initialized. Startup registration is provided by Plan 003.'
    }
    Write-Host "Run when available: `"$ServerBinary`" startup install"
}

function Install-Component([string]$Name, [string]$RequestedVersion) {
    $Package = Get-Package $Name
    $Binary = Get-Binary $Name
    $SelectedVersion = $RequestedVersion
    if (-not $SelectedVersion) { $SelectedVersion = Get-CrateVersion $Package }
    else { Test-StableVersion $SelectedVersion }

    $Target = Get-Target
    $Temp = Join-Path ([System.IO.Path]::GetTempPath()) ("snip-it-install-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $Temp | Out-Null
    try {
        $Candidate = $null
        if ($Target -eq 'source-only' -or $Target -eq 'aarch64-pc-windows-msvc') {
            Write-Host "$Binary ${SelectedVersion}: target $Target is source-only; using Cargo fallback."
            $Candidate = Get-CargoCandidate $Name $SelectedVersion (Join-Path $Temp 'cargo-root') $Target
        }
        else {
            $Asset = "$Binary-$Target.exe"
            $Tag = Get-Tag $Name $SelectedVersion
            $Candidate = Join-Path $Temp $Asset
            $Checksum = "$Candidate.sha256"
            $assetUri = "$(Get-GitHubBase)/$Tag/$Asset"
            if (Invoke-Download $assetUri $Candidate -AllowNotFound) {
                if (-not (Invoke-Download "$assetUri.sha256" $Checksum)) {
                    throw "Checksum download failed for $Asset; refusing Cargo fallback"
                }
                Assert-Candidate $Candidate $Checksum $Asset $Name $SelectedVersion
            }
            else {
                Write-Host "$Binary ${SelectedVersion}: release asset $Asset is unavailable; using Cargo fallback."
                $Candidate = Get-CargoCandidate $Name $SelectedVersion (Join-Path $Temp 'cargo-root') $Target
            }
        }

        $Destination = Get-Destination
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        Copy-Item -LiteralPath $Candidate -Destination (Join-Path $Destination "$Binary.exe") -Force
        $Installed = Join-Path $Destination "$Binary.exe"
        Write-Host "Installed $Binary $SelectedVersion at $Installed"
        if (-not (Test-PathEntry $Destination)) {
            Write-Host "Add $Destination to PATH to run $Binary directly."
        }
        if ($Name -eq 'Server') { Install-ServerSetup $Installed }
    }
    finally {
        if (Test-Path -LiteralPath $Temp) { Remove-Item -LiteralPath $Temp -Recurse -Force }
    }
}

if ($Version) { Test-StableVersion $Version }
if ($Component -eq 'Both' -and $Version) {
    throw '-Version is ambiguous with -Component Both; install each component separately'
}

if ($Component -eq 'Both') {
    Install-Component 'Snp' $null
    Install-Component 'Server' $null
}
else {
    Install-Component $Component $Version
}
