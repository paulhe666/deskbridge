$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Dist = Join-Path $Root "dist\windows"
$InnoScript = Join-Path $Root "packaging\windows\deskbridge.iss"

Set-Location $Root
if (Test-Path (Join-Path $Root "web\package.json")) {
    Push-Location (Join-Path $Root "web")
    npm install
    npm run build
    Pop-Location
}

$FrontendIndex = Join-Path $Root "web\dist\index.html"
if (-not (Test-Path $FrontendIndex)) {
    throw "Missing web\dist\index.html. Windows production builds must include the built frontend assets."
}

cargo build --release

$ReleaseExe = Join-Path $Root "target\release\deskbridge.exe"
if (-not (Test-Path $ReleaseExe)) {
    throw "Missing target\release\deskbridge.exe after cargo build."
}

New-Item -ItemType Directory -Force -Path $Dist | Out-Null

$Iscc = Get-Command iscc.exe -ErrorAction SilentlyContinue
if (-not $Iscc) {
    $Candidate = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    if (Test-Path $Candidate) {
        $Iscc = Get-Item $Candidate
    }
}

if (-not $Iscc) {
    Write-Host "Inno Setup 6 was not found. Install it from https://jrsoftware.org/isinfo.php and rerun this script."
    Write-Host "The release binary is ready at: $Root\target\release\deskbridge.exe"
    exit 1
}

& $Iscc.Source $InnoScript

Write-Host "Created installer in $Dist"
