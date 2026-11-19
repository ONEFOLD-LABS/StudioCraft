<#
.SYNOPSIS
  Authenticode-sign files with signtool when a certificate is configured.

.DESCRIPTION
  Uses WINDOWS_CERTIFICATE (base64 .pfx) and WINDOWS_CERTIFICATE_PASSWORD. Without them it prints
  a warning and leaves the files unsigned (exit code 0), so test builds still produce installers.
  Optional: WINDOWS_TIMESTAMP_URL, SIGNTOOL (path to signtool.exe).
  Adapted from SoundCraft (storytold/soundcraft, Apache-2.0), see NOTICE.

.EXAMPLE
  pwsh packaging/windows/sign.ps1 dist/studiocraft.exe dist/studiocraft.msi
#>
param(
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
  [string[]] $Files
)
$ErrorActionPreference = 'Stop'

function Write-Warn([string] $Message) {
  if ($env:GITHUB_ACTIONS) { Write-Output "::warning::$Message" } else { Write-Warning $Message }
}

if (-not $env:WINDOWS_CERTIFICATE) {
  Write-Warn "WINDOWS_CERTIFICATE not set: leaving unsigned: $($Files -join ', ')"
  exit 0
}

function Find-SignTool {
  if ($env:SIGNTOOL) { return $env:SIGNTOOL }
  $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
  $found = Get-ChildItem -Path $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Sort-Object FullName -Descending | Select-Object -First 1
  if (-not $found) { throw 'signtool.exe not found (install the Windows SDK)' }
  return $found.FullName
}

$SignTool = Find-SignTool
$Files = $Files | ForEach-Object { (Resolve-Path $_).Path }
$tmp = Join-Path ([IO.Path]::GetTempPath()) "studiocraft-sign-$([IO.Path]::GetRandomFileName())"
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $pfx = Join-Path $tmp 'cert.pfx'
  [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE))
  $ts = if ($env:WINDOWS_TIMESTAMP_URL) { $env:WINDOWS_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
  $signArgs = @('sign', '/v', '/fd', 'SHA256', '/td', 'SHA256', '/d', 'StudioCraft', '/tr', $ts, '/f', $pfx)
  if ($env:WINDOWS_CERTIFICATE_PASSWORD) { $signArgs += @('/p', $env:WINDOWS_CERTIFICATE_PASSWORD) }
  & $SignTool @signArgs @Files
  if ($LASTEXITCODE -ne 0) { throw "signtool sign failed with exit code $LASTEXITCODE" }
  & $SignTool verify /pa /v @Files
  if ($LASTEXITCODE -ne 0) { throw "signtool verify failed with exit code $LASTEXITCODE" }
}
finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
