param(
    [ValidateSet("Steam", "Epic", "All")]
    [string]$Target = "All",
    [ValidateSet("Shipping", "Development", "DebugGame")]
    [string]$Config = "Shipping",
    [switch]$SkipCook,
    [switch]$Clean,
    [switch]$Iterate,
    [switch]$CopySteamAppId,
    [string]$Platform = "Win64"
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$UProjectFile = Get-ChildItem -Path $ProjectRoot -Filter "*.uproject" | Select-Object -First 1
if (-not $UProjectFile) { Write-Error "No .uproject file found"; exit 1 }

$UProjectPath = $UProjectFile.FullName
$ProjectName = $UProjectFile.BaseName
$EngineRoot = $null

if ($env:UE_ROOT) {
    $EngineRoot = $env:UE_ROOT
} else {
    $UProject = Get-Content $UProjectPath | ConvertFrom-Json
    $v = $UProject.EngineAssociation
    @("W:\Softwares\UE_$v", "C:\Program Files\Epic Games\UE_$v", "D:\Softwares\UE_$v") | ForEach-Object {
        if ((Test-Path "$_\Engine\Build\BatchFiles\RunUAT.bat") -and -not $EngineRoot) { $EngineRoot = $_ }
    }
}

if (-not $EngineRoot) { Write-Error "Cannot find UE engine. Set UE_ROOT env var."; exit 1 }

$RunUAT = "$EngineRoot\Engine\Build\BatchFiles\RunUAT.bat"
$OutputBase = "$ProjectRoot\Build"

Write-Host "OpenWorldFramework Packaging" -ForegroundColor Cyan
Write-Host "  Project: $UProjectPath" -ForegroundColor Gray
Write-Host "  Engine:  $EngineRoot" -ForegroundColor Gray
Write-Host "  Target:  $Target | Config: $Config | Platform: $Platform" -ForegroundColor Gray

$Targets = @()
switch ($Target) {
    "Steam" { $Targets = @("Steam") }
    "Epic"  { $Targets = @("Epic") }
    "All"   { $Targets = @("Steam", "Epic") }
}

$TargetMap = @{
    "Steam" = @{ TargetName = "${ProjectName}Steam"; OutputDir = "$OutputBase\Steam"; Label = "Steam" }
    "Epic"  = @{ TargetName = "${ProjectName}Epic";  OutputDir = "$OutputBase\Epic";  Label = "Epic Games Store" }
}

if (-not $SkipCook) {
    Write-Host "`nStep 1: Cooking content (shared across all targets)..." -ForegroundColor Yellow
    $CookArgs = @(
        "BuildCookRun",
        "-project=`"$UProjectPath`"",
        "-noP4", "-WaitMutex", "-platform=$Platform", "-clientconfig=$Config",
        "-cook", "-allmaps", "-unversionedcookedcontent",
        "-pak", "-compressed",
        "-stage", "-stagingdirectory=`"$OutputBase\Cooked`""
    )
    if ($Iterate) { $CookArgs += "-iterate" }
    if ($Clean)   { $CookArgs += "-clean" }

    $t0 = Get-Date
    & $RunUAT @CookArgs
    if ($LASTEXITCODE -ne 0) { Write-Host "COOK FAILED" -ForegroundColor Red; exit 1 }
    Write-Host "Content cooked in $((Get-Date).Subtract($t0).ToString('mm\:ss'))" -ForegroundColor Green
} else {
    Write-Host "`nSkipping cook (reusing existing content)" -ForegroundColor Yellow
}

$step = 2
foreach ($T in $Targets) {
    $Info = $TargetMap[$T]
    Write-Host "`nStep ${step}: Building $($Info.Label)..." -ForegroundColor Yellow

    $BuildArgs = @(
        "BuildCookRun",
        "-project=`"$UProjectPath`"",
        "-noP4", "-WaitMutex", "-platform=$Platform", "-clientconfig=$Config",
        "-target=$($Info.TargetName)",
        "-build", "-skipcook",
        "-pak", "-compressed",
        "-stage", "-stagingdirectory=`"$($Info.OutputDir)`""
    )

    $t0 = Get-Date
    & $RunUAT @BuildArgs
    if ($LASTEXITCODE -ne 0) { Write-Host "BUILD FAILED for $($Info.Label)" -ForegroundColor Red; exit 1 }
    Write-Host "$($Info.Label) built in $((Get-Date).Subtract($t0).ToString('mm\:ss'))" -ForegroundColor Green
    $step++
}

# Copy steam_appid.txt if requested (for dev/QA testing — never ship to customers)
if ($CopySteamAppId) {
    $SteamAppIdFile = "$ProjectRoot\steam_appid.txt"
    if (Test-Path $SteamAppIdFile) {
        foreach ($T in $Targets) {
            $ExeDir = "$($TargetMap[$T].OutputDir)\Windows"
            if (Test-Path $ExeDir) {
                Copy-Item $SteamAppIdFile "$ExeDir\steam_appid.txt" -Force
                Write-Host "  Copied steam_appid.txt to $ExeDir (dev testing only)" -ForegroundColor DarkYellow
            }
        }
    } else {
        Write-Host "  steam_appid.txt not found in project root — skipping" -ForegroundColor DarkYellow
    }
}

Write-Host "`nPackaging complete!" -ForegroundColor Green
foreach ($T in $Targets) { Write-Host "  $($TargetMap[$T].Label): $($TargetMap[$T].OutputDir)" -ForegroundColor White }
