<#
.SYNOPSIS
  Build, sign and package StudioCraft for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    studiocraft-<version>-windows-<arch>.msi            per-machine installer (WiX v5)
    studiocraft-<version>-windows-<arch>-portable.zip   studiocraft.exe + studiocraft-cli.exe

  The binaries link the C runtime statically (+crt-static), so neither package needs the Visual
  C++ redistributable. Signing is delegated to sign.ps1 (skipped with a warning without secrets).
  Release builds use WASAPI only; ASIO is not part of the packages (see issue #107).

  Needs: Rust (MSVC toolchain + the target), the Windows SDK (rc.exe), and WiX v5:
    dotnet tool install --global wix --version 5.0.2
  Adapted from SoundCraft (storytold/soundcraft, Apache-2.0), see NOTICE.

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
  pwsh packaging/windows/package.ps1 -Arch arm64     # cross-compiled; needs the MSVC ARM64 tools
#>
param(
  [ValidateSet('x64', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = (cargo xtask version | Select-Object -Last 1).Trim()
if (-not $Version) { throw 'cargo xtask version printed nothing' }
# MSI ProductVersion is numeric (major.minor.build); pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

Write-Output "StudioCraft $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT, scoped to the target so build scripts and proc-macros are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  # Fail (rather than warn) if the icon and version info can't be embedded.
  $env:STUDIOCRAFT_REQUIRE_WINRES = '1'
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p studiocraft -p studiocraft-cli --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\release"

# PE header checks: the machine must match -Arch, the app must be a GUI program (subsystem 2, no
# console window) and the CLI a console program (subsystem 3) so its output reaches the terminal.
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'arm64' { 0xAA64 } }
foreach ($check in @(@('studiocraft.exe', 2), @('studiocraft-cli.exe', 3))) {
  $h = Get-PeHeader (Join-Path $Bin $check[0])
  if ($h.Machine -ne $Machine) { throw "$($check[0]) is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
  if ($h.Subsystem -ne $check[1]) { throw "$($check[0]) has PE subsystem $($h.Subsystem), expected $($check[1])" }
  Write-Output "ok $($check[0]): $Arch, PE subsystem $($h.Subsystem)"
}

$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin 'studiocraft.exe'), (Join-Path $Bin 'studiocraft-cli.exe') $Stage
foreach ($f in 'NOTICE', 'LICENSE-MIT', 'LICENSE-APACHE') { Copy-Item (Join-Path $Root $f) $Stage }

# Smoke tests where this machine can run the build. The app is a GUI program: its stdout must
# still reach a pipe, because the plugin scan runs it as a child process and reads the result
# from stdout.
$HostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($Arch -ne 'arm64' -or $HostArch -eq 'arm64') {
  Invoke-Native 'studiocraft-cli --version' { & (Join-Path $Stage 'studiocraft-cli.exe') --version }
  $out = (& (Join-Path $Stage 'studiocraft.exe') --version | Out-String).Trim()
  if ($LASTEXITCODE -ne 0 -or $out -notmatch '^studiocraft ') { throw "studiocraft.exe --version through a pipe gave '$out' (exit $LASTEXITCODE)" }
  Write-Output "ok studiocraft.exe stdout through a pipe: $out"
} else {
  Write-Output "skipping smoke tests: an $Arch build doesn't run on this $HostArch machine"
}

& (Join-Path $PSScriptRoot 'sign.ps1') (Join-Path $Stage 'studiocraft.exe') (Join-Path $Stage 'studiocraft-cli.exe')

# ---- MSI ---------------------------------------------------------------------------------------
$Msi = Join-Path $Dist "studiocraft-$Version-windows-$Arch.msi"
Invoke-Native 'WiX UI extension' { wix extension add -g WixToolset.UI.wixext/5.0.2 }
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'studiocraft.wxs') -arch $Arch -ext WixToolset.UI.wixext `
    -d "Version=$MsiVersion" -d "BinDir=$Stage" -d "IconPath=$(Join-Path $Root 'assets\app-icon\studiocraft.ico')" `
    -o $Msi
}
# wix writes debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') $Msi

# ---- portable zip ------------------------------------------------------------------------------
$Portable = Join-Path $TargetDir "windows-package\studiocraft-$Version-windows-$Arch-portable"
Remove-Item -Recurse -Force $Portable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Portable | Out-Null
Copy-Item (Join-Path $Stage '*') $Portable
Copy-Item (Join-Path $Root 'README.md') $Portable
$Zip = Join-Path $Dist "studiocraft-$Version-windows-$Arch-portable.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Portable -DestinationPath $Zip

Get-Item $Msi, $Zip | Format-Table Name, Length
