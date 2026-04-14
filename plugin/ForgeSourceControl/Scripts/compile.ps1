# compile.ps1 - Unified Unreal Engine build script
#
# Modular: drop into any UE project root. Auto-detects project from .uproject file.
# Requires UE_ROOT environment variable (or falls back to default with warning).
#
# If the editor is running (LiveCodingRemote HTTP server responds), triggers
# Live Coding and returns structured results.
# If the editor is closed, falls back to UnrealBuildTool.
#
# Exit code: 0 on success, 1 on failure

param(
    [int]$Port = 1800,
    [int]$TimeoutSec = 300
)

$ErrorActionPreference = "Stop"

# ── Resolve project root and .uproject ────────────────────────────────────────

$ProjectRoot = Split-Path $PSScriptRoot -Parent

$UProjectFile = Get-ChildItem -Path $ProjectRoot -Filter "*.uproject" -File | Select-Object -First 1
if (-not $UProjectFile) {
    Write-Host "No .uproject file found in $ProjectRoot" -ForegroundColor Red
    exit 1
}

$ProjectName = [System.IO.Path]::GetFileNameWithoutExtension($UProjectFile.Name) + "Editor"
$ProjectPath = $UProjectFile.FullName

# ── Resolve UE_ROOT ───────────────────────────────────────────────────────────

$UE_ROOT = if ($env:UE_ROOT) { $env:UE_ROOT.Trim() } else {
    Write-Host "WARNING: UE_ROOT environment variable not set, using default: C:\Program Files\Epic Games\UE_5.7" -ForegroundColor Yellow
    "C:\Program Files\Epic Games\UE_5.7"
}

# ── Try Live Coding first ────────────────────────────────────────────────────

$liveCodingAvailable = $false
try {
    $response = Invoke-RestMethod -Uri "http://localhost:$Port/compile" -Method Post -TimeoutSec $TimeoutSec
    $liveCodingAvailable = $true
}
catch {
    # Connection refused or timeout — editor not running
}

if ($liveCodingAvailable) {
    if ($response.success) {
        Write-Host "Build succeeded ($($response.status))" -ForegroundColor Green
        exit 0
    } else {
        Write-Host "Build failed ($($response.status))" -ForegroundColor Red
        if ($response.logs) {
            Write-Host ""
            foreach ($line in $response.logs) {
                Write-Host "  $line" -ForegroundColor Red
            }
        }
        exit 1
    }
}

# ── Fallback: UnrealBuildTool (editor closed) ────────────────────────────────

Write-Host "Editor not running, using UnrealBuildTool..." -ForegroundColor Yellow
Write-Host ""

$BuildBat = Join-Path $UE_ROOT "Engine\Build\BatchFiles\Build.bat"

if (-not (Test-Path $BuildBat)) {
    Write-Host "Build.bat not found at: $BuildBat" -ForegroundColor Red
    Write-Host "Set UE_ROOT environment variable to your Unreal Engine installation." -ForegroundColor Red
    exit 1
}

$output = & cmd.exe /c "`"$BuildBat`" $ProjectName Win64 Development -Project=`"$ProjectPath`" -WaitMutex -FromMsBuild -architecture=x64" 2>&1 | Out-String

if ($output -match "Result: Succeeded") {
    Write-Host "Build succeeded" -ForegroundColor Green
    exit 0
} else {
    $output -split "`n" | Where-Object {
        $_ -match "error |fatal |Result:|Unable to build"
    } | ForEach-Object { Write-Host $_.Trim() -ForegroundColor Red }
    Write-Host ""
    Write-Host "Build failed" -ForegroundColor Red
    exit 1
}
