# Requires -Version 5.1
$ErrorActionPreference = "Stop"

$Repo = "neon-tasker/phalanx"
$BinaryName = "phalanx-daemon.exe"
$AliasName = "phalanx.exe"
$InstallDir = "$env:USERPROFILE\.phalanx\bin"
$Target = "x86_64-pc-windows-msvc"

function Write-PhalanxInfo($msg) {
    Write-Host "==> " -ForegroundColor Green -NoNewline
    Write-Host $msg
}
function Write-PhalanxWarn($msg) {
    Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline
    Write-Host $msg
}
function Write-PhalanxError($msg) {
    Write-Host "[ERROR] " -ForegroundColor Red -NoNewline
    Write-Host $msg
    exit 1
}

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($Arch -ne [System.Runtime.InteropServices.Architecture]::X64) {
    Write-PhalanxError "Phalanx Daemon Windows distribution requires x86_64 (AMD64) architecture."
}

if ($env:PHALANX_VERSION) {
    $Version = $env:PHALANX_VERSION
    Write-PhalanxInfo "Using explicit version: $Version"
} else {
    Write-PhalanxInfo "Resolving latest release tag from GitHub..."
    try {
        $Req = [System.Net.WebRequest]::Create("https://github.com/$Repo/releases/latest")
        $Req.AllowAutoRedirect = $false
        $Req.Method = "HEAD"
        $Response = $Req.GetResponse()
        $Location = $Response.GetResponseHeader("Location")
        $Response.Close()

        if ($Location) {
            $Version = $Location.Substring($Location.LastIndexOf('/') + 1)
        }
    } catch {
        Write-PhalanxWarn "Redirect query failed, falling back to API..."
    }

    if (-not $Version) {
        try {
            $ApiUri = "https://api.github.com/repos/$Repo/releases/latest"
            $ApiRelease = Invoke-RestMethod -Uri $ApiUri -Headers @{ "User-Agent" = "Phalanx-Installer" }
            $Version = $ApiRelease.tag_name
        } catch {
            Write-PhalanxError "Failed to resolve release version. Set `$env:PHALANX_VERSION manually."
        }
    }
}
Write-PhalanxInfo "Target version: $Version"

$ZipName = "phalanx-daemon-$Version-$Target.zip"
$ChecksumName = "$ZipName.sha256"
$BaseUrl = "https://github.com/$Repo/releases/download/$Version"

$TempDir = Join-Path $env:TEMP ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $TempDir -Force | Out-Null
$ZipPath = Join-Path $TempDir $ZipName
$ChecksumPath = Join-Path $TempDir $ChecksumName

try {
    Write-PhalanxInfo "Downloading $ZipName..."
    Invoke-WebRequest -Uri "$BaseUrl/$ZipName" -OutFile $ZipPath -UseBasicParsing

    Write-PhalanxInfo "Downloading $ChecksumName..."
    Invoke-WebRequest -Uri "$BaseUrl/$ChecksumName" -OutFile $ChecksumPath -UseBasicParsing
} catch {
    Write-PhalanxError "Download failed: $_"
}

Write-PhalanxInfo "Validating cryptographic SHA256 signature..."
$ExpectedHash = (Get-Content $ChecksumPath).Split(" ")[0].Trim().ToLower()
$ActualHash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()

if ($ExpectedHash -ne $ActualHash) {
    Write-PhalanxError "SHA256 hash mismatch! Expected: $ExpectedHash, Received: $ActualHash"
}

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

Expand-Archive -Path $ZipPath -DestinationPath $TempDir -Force
$ExtractedBin = Join-Path $TempDir "phalanx-daemon.exe"

if (-not (Test-Path $ExtractedBin)) {
    Write-PhalanxError "Archive payload did not contain phalanx-daemon.exe."
}

Copy-Item -Path $ExtractedBin -Destination (Join-Path $InstallDir $BinaryName) -Force
Copy-Item -Path $ExtractedBin -Destination (Join-Path $InstallDir $AliasName) -Force

Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue

Write-PhalanxInfo "Configuring persistent User Environment PATH..."
$CurrentPath = [Environment]::GetEnvironmentVariable("PATH", [EnvironmentVariableTarget]::User)
if ($CurrentPath -notlike "*$InstallDir*") {
    $NewPath = "$InstallDir;$CurrentPath".TrimEnd(';')
    [Environment]::SetEnvironmentVariable("PATH", $NewPath, [EnvironmentVariableTarget]::User)
    $env:PATH = "$InstallDir;$env:PATH"
    Write-PhalanxInfo "Added $InstallDir to User PATH."
} else {
    $env:PATH = "$InstallDir;$env:PATH"
}

$Executable = Join-Path $InstallDir $BinaryName
if (Test-Path $Executable) {
    Write-Host @"

    ____  __  _____    __    ___    _   ___  __
   / __ \/ / / /   |  / /   /   |  / | / / |/ /
  / /_/ / /_/ / /| | / /   / /| | /  |/ /|   / 
 / ____/ __  / ___ |/ /___/ ___ |/ /|  //   |  
/_/   /_/ /_/_/  |_/_____/_/  |_/_/ |_//_/|_|  
           IN-MEMORY REVM SIMULATION DAEMON
"@ -ForegroundColor Cyan

    Write-Host "`nPhalanx Daemon ($Version) deployed successfully!" -ForegroundColor Green
    Write-Host "Staged Path: $Executable" -ForegroundColor White
    Write-Host "CLI Alias:   $(Join-Path $InstallDir $AliasName)" -ForegroundColor White
    Write-Host "`nRun 'phalanx --help' or 'phalanx-daemon verify' to begin.`n" -ForegroundColor Yellow
} else {
    Write-PhalanxError "Installation verification failed. Executable not found."
}