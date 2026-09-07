# Avvia DSH Desktop Manager in modalità sviluppo.
# Configura automaticamente l'ambiente MSVC di Visual Studio (per cargo/link),
# poi lancia Vite + l'app Tauri. Si apre una finestra: chiudila per terminare.
$ErrorActionPreference = "Stop"

$vsCandidates = @(
    "C:\Program Files\Microsoft Visual Studio\2022\Community",
    "C:\Program Files\Microsoft Visual Studio\2022\Professional",
    "C:\Program Files\Microsoft Visual Studio\2022\Enterprise",
    "C:\Program Files\Microsoft Visual Studio\2022\BuildTools"
)
$vs = $vsCandidates | Where-Object { Test-Path (Join-Path $_ "VC\Tools\MSVC") } | Select-Object -First 1
if (-not $vs) {
    throw "Visual Studio 2022 con i componenti C++ non trovato."
}

$msvc = Get-ChildItem (Join-Path $vs "VC\Tools\MSVC") -Directory |
    Sort-Object Name -Descending | Select-Object -First 1
$sdkRoot = "C:\Program Files (x86)\Windows Kits\10"
$sdk = Get-ChildItem (Join-Path $sdkRoot "Lib") -Directory |
    Sort-Object Name -Descending | Select-Object -First 1
if (-not $sdk) {
    throw "Windows SDK non trovato in $sdkRoot"
}

$bin = Join-Path $msvc.FullName "bin\Hostx64\x64"
$env:PATH = "$bin;$env:PATH"
$env:LIB = "$($sdk.FullName)\ucrt\x64;$($sdk.FullName)\um\x64;$($msvc.FullName)\lib\x64"
$env:INCLUDE = "$($sdk.FullName)\ucrt\include;$($sdk.FullName)\um\include;$($sdk.FullName)\shared\include;$($msvc.FullName)\include"

Set-Location $PSScriptRoot
Write-Host ""
Write-Host "Avvio DSH Desktop Manager (dev)..."
Write-Host "La prima compilazione può richiedere qualche minuto."
Write-Host "Chiudi la finestra dell'app (o premi Ctrl+C qui) per terminare."
Write-Host ""
npm run tauri dev
