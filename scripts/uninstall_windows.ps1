$ErrorActionPreference = "Stop"

$InstallDir = Join-Path $env:ProgramFiles "Deskbridge"
$ConfigDir = Join-Path $env:USERPROFILE ".deskbridge"

Get-Process deskbridge -ErrorAction SilentlyContinue | Stop-Process -Force

$Uninstaller = Join-Path $InstallDir "unins000.exe"
if (Test-Path $Uninstaller) {
    & $Uninstaller /VERYSILENT
} else {
    Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue
}

Remove-Item -Recurse -Force $ConfigDir -ErrorAction SilentlyContinue
Write-Host "Deskbridge has been removed."
