$ErrorActionPreference = "Stop"
$Repo = "phalanx-engine/phalanx"
$BinaryName = "phalanx-daemon.exe"
$InstallDir = "$env:USERPROFILE\.phalanx\bin"
$DefaultVersion = "v0.2.0"

Write-Host "==> Detecting Windows Architecture..." -ForegroundColor Green
$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($Arch -ne [System.Runtime.InteropServices.Architecture]::X64) {
    Write-Error "[ERROR] Phalanx Daemon on Windows natively supports x64 (x86_64) architecture only."
    exit 1
}

$Target = "x86_64-pc-windows-msvc"
Write-Host "==> Querying latest release for target $Target..." -ForegroundColor Green
try {
    $ReleaseUri = "https://api.github.com/repos/$Repo/releases/latest"
    $Release = Invoke-RestMethod -Uri $ReleaseUri -Headers @{ "User-Agent" = "PowerShell-Phalanx-Installer" }
    $Version = $Release.tag_name
} catch {
    Write-Warning "Could not connect to GitHub Releases API. Defaulting to $DefaultVersion."
    $Version = $DefaultVersion
}

$ZipName = "phalanx-daemon-$Version-$Target.zip"
$DownloadUrl = "https://github.com/$Repo/releases/download/$Version/$ZipName"
$TempZip = Join-Path $env:TEMP $ZipName

Write-Host "==> Downloading $DownloadUrl..." -ForegroundColor Green
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempZip -UseBasicParsing
} catch {
    Write-Error "[ERROR] Failed to download release asset from $DownloadUrl. Check connection or release status."
    exit 1
}

if (!(Test-Path -Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

Write-Host "==> Extracting binary to $InstallDir..." -ForegroundColor Green
Expand-Archive -Path $TempZip -DestinationPath $env:TEMP -Force
$ExtractedBin = Join-Path $env:TEMP "phalanx-daemon.exe"
if (Test-Path $ExtractedBin) {
    Move-Item -Path $ExtractedBin -Destination (Join-Path $InstallDir $BinaryName) -Force
    Remove-Item $TempZip -Force -ErrorAction SilentlyContinue
} else {
    Write-Error "[ERROR] Extracted payload did not contain phalanx-daemon.exe."
    exit 1
}

Write-Host "==> Configuring User Environment PATH..." -ForegroundColor Green
$CurrentPath = [Environment]::GetEnvironmentVariable("PATH", [EnvironmentVariableTarget]::User)
if ($CurrentPath -notlike "*$InstallDir*") {
    $NewPath = "$CurrentPath;$InstallDir"
    [Environment]::SetEnvironmentVariable("PATH", $NewPath, [EnvironmentVariableTarget]::User)
    $env:PATH = "$env:PATH;$InstallDir"
    Write-Host "Added $InstallDir to User Environment PATH." -ForegroundColor Green
} else {
    $env:PATH = "$env:PATH;$InstallDir"
}

$ExecutablePath = Join-Path $InstallDir $BinaryName
if (Test-Path $ExecutablePath) {
    Write-Host @"

    ____  __  _____    __    ___    _   ___  __
   / __ \/ / / /   |  / /   /   |  / | / / |/ /
  / /_/ / /_/ / /| | / /   / /| | /  |/ /|   / 
 / ____/ __  / ___ |/ /___/ ___ |/ /|  //   |  
/_/   /_/ /_/_/  |_/_____/_/  |_/_/ |_//_/|_|  
           IN-MEMORY REVM SIMULATION DAEMON
"@ -ForegroundColor Cyan

    Write-Host "`nPhalanx Daemon ($Version) installed successfully to: $ExecutablePath" -ForegroundColor Green
    Write-Host "Run 'phalanx-daemon --help' to start." -ForegroundColor Yellow
} else {
    Write-Error "[ERROR] Installation verification failed: $ExecutablePath not found."
    exit 1
}