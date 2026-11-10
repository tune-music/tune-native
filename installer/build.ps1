# Собирает Tune Native и установщик: target/installer/Tune-Native-<версия>-setup.exe.
# NSIS — тот, что ставит Tauri (%LOCALAPPDATA%\tauri\NSIS), или путь к makensis.exe в $env:MAKENSIS.
#
#   pwsh -File installer/build.ps1            # сборка программы и установщика
#   pwsh -File installer/build.ps1 -NoBuild   # только установщик из готовой сборки
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not (Select-String -Path CHANGES.md -Pattern "^## $([regex]::Escape($version))$" -Quiet)) {
    throw "В CHANGES.md нет раздела ## $version"
}
if (-not $NoBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'Сборка не удалась' }
}
$release = Join-Path $root 'target\release'
if (-not (Test-Path "$release\libmpv-2.dll")) {
    Write-Warning 'Нет libmpv-2.dll: установщик соберётся без клипов и живого фона'
}

$makensis = $env:MAKENSIS
if (-not $makensis) { $makensis = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\Bin\makensis.exe' }
if (-not (Test-Path $makensis)) { throw "Нет makensis.exe: $makensis" }

$out = Join-Path $root 'target\installer'
New-Item -ItemType Directory -Force $out | Out-Null
$setup = Join-Path $out "Tune-Native-$version-setup.exe"
& $makensis /V2 /INPUTCHARSET UTF8 "/DVERSION=$version" "/DRELEASE=$release" "/DOUTFILE=$setup" (Join-Path $PSScriptRoot 'tune-native.nsi')
if ($LASTEXITCODE -ne 0) { throw 'makensis не собрал установщик' }
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
"{0}  {1:N1} МБ  sha256 {2}" -f $setup, ((Get-Item $setup).Length / 1MB), $hash
