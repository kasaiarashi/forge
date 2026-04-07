<#
    Clean.ps1
    ---------------------------------------
    Unreal Engine project cleanup script

    Deletes:
    - Binaries
    - Intermediate
    - Saved
    - DerivedDataCache
    - node_modules (ONLY inside Docs folder)

    Scope:
    - Project root
    - All plugins
    - Nested subfolders

    Usage:
    - Run from project root
    - Or: powershell -ExecutionPolicy Bypass -File Clean.ps1
#>

Write-Host "========================================" -ForegroundColor Cyan
Write-Host " Unreal Engine Project Cleanup Started " -ForegroundColor Cyan
Write-Host "========================================`n" -ForegroundColor Cyan

$Targets = @(
    "Binaries",
    "Intermediate",
    "Saved",
    "DerivedDataCache"
)

$ProjectRoot = Split-Path $PSScriptRoot -Parent
$Deleted = 0

Get-ChildItem -Path $ProjectRoot -Directory -Recurse -ErrorAction SilentlyContinue |
Where-Object {
    ($Targets -contains $_.Name) -or
    ($_.Name -eq "node_modules" -and $_.FullName -like "*\Docs\*")
} |
ForEach-Object {
    try {
        Write-Host "🗑️  Removing $($_.FullName)" -ForegroundColor Yellow
        Remove-Item $_.FullName -Recurse -Force -ErrorAction Stop
        $Deleted++
    }
    catch {
        Write-Host "⚠️  Failed to remove $($_.FullName)" -ForegroundColor Red
    }
}

Write-Host "`n----------------------------------------" -ForegroundColor Cyan
Write-Host " Cleanup Complete" -ForegroundColor Green
Write-Host " Folders Removed: $Deleted" -ForegroundColor Green
Write-Host "----------------------------------------" -ForegroundColor Cyan