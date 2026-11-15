<#
.SYNOPSIS
  Build, sign and package the CraftCloud desktop client for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    craftcloud-<version>-windows-<arch>.msi            per-user installer with a setup wizard (WiX v5)
    craftcloud-<version>-windows-<arch>-portable.zip   craftcloud.exe, craftcloud.com and the notices, to run from anywhere

  The exe keeps the name craftcloud.exe: it is also the command line (craftcloud status). Being a
  GUI program, it gets no console, so craftcloud.com (the craftcloud-cli package, a console
  program) sits next to it and runs it for a terminal: Command Prompt and PowerShell try .com
  before .exe. Both link the C runtime statically (.cargo/config.toml), so neither file needs the Visual C++
  redistributable. Signing goes through sign.ps1, which leaves the files unsigned with a warning
  when no signing secrets are set.

  Needs: Rust (MSVC toolchain + the target), the Windows SDK (signtool.exe, only for signing),
  and WiX v5: dotnet tool install --global wix --version 5.0.2
             wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2

  Adapted from craft-launcher's packaging/windows/package.ps1 (MIT OR Apache-2.0; see NOTICE),
  with WiX's own wizard art.

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
  pwsh packaging/windows/package.ps1 -Arch arm64 -SkipBuild
#>
param(
  [ValidateSet('x64', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

$Pkg = 'craftcloud'   # cargo package and binary
$CliPkg = 'craftcloud-cli'   # cargo package and binary of the terminal launcher, shipped as craftcloud.com
$AppName = 'CraftCloud'

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = $env:CRAFTCLOUD_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
# MSI ProductVersion is numeric (major.minor.build); pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

Write-Output "$AppName $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # .cargo/config.toml links the C runtime statically for both MSVC targets.
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p $Pkg -p $CliPkg --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\release"
$Exe = "$Pkg.exe"
$Com = "$Pkg.com"

# Check the PE header before packaging. Machine (COFF header) must match -Arch, so an x64 build
# can never ship labelled arm64. Subsystem (optional header) must be 2 = Windows GUI for the client,
# so no console window opens with it, and 3 = Windows console for the launcher, so a terminal waits
# for it and shows its output.
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'arm64' { 0xAA64 } }
$h = Get-PeHeader (Join-Path $Bin $Exe)
if ($h.Machine -ne $Machine) { throw "$Exe is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
if ($h.Subsystem -ne 2) { throw "$Exe has PE subsystem $($h.Subsystem), expected 2 (Windows GUI)" }
Write-Output "ok ${Exe}: $Arch, PE subsystem $($h.Subsystem)"
$h = Get-PeHeader (Join-Path $Bin "$CliPkg.exe")
if ($h.Machine -ne $Machine) { throw "$CliPkg.exe is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
if ($h.Subsystem -ne 3) { throw "$CliPkg.exe has PE subsystem $($h.Subsystem), expected 3 (Windows console)" }
Write-Output "ok ${CliPkg}.exe (as $Com): $Arch, PE subsystem $($h.Subsystem)"

$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item -LiteralPath (Join-Path $Root 'NOTICE') -Destination $Stage
Copy-Item (Join-Path $Bin $Exe) $Stage
# Windows runs a .com by its PE header, like an .exe; the name only puts it first for `craftcloud`.
Copy-Item (Join-Path $Bin "$CliPkg.exe") (Join-Path $Stage $Com)

& (Join-Path $PSScriptRoot 'sign.ps1') (Join-Path $Stage $Exe) (Join-Path $Stage $Com)

# ---- MSI ---------------------------------------------------------------------------------------
$Msi = Join-Path $Dist "$Pkg-$Version-windows-$Arch.msi"
$Icon = Join-Path $Root 'apps\craftcloud\assets\craftcloud.ico'
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot "$Pkg.wxs") -arch $Arch `
    -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -culture en-US -loc (Join-Path $PSScriptRoot "$Pkg.en-us.wxl") `
    -d "Version=$MsiVersion" -d "DisplayVersion=$Version" -d "BinDir=$Stage" -d "IconPath=$Icon" `
    -o $Msi
}
Invoke-Native 'MSI shortcut icon validation (ICE50)' {
  wix msi validate $Msi -ice ICE50 -intermediateFolder (Join-Path $Stage 'msi-validation')
}
# wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release files.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') $Msi

# ---- portable zip ------------------------------------------------------------------------------
# The same two programs without an installer (not on PATH: `craftcloud` works in that folder, or
# add it to PATH by hand). It keeps its settings in %APPDATA% like the installed one,
# and registers craftcloud:// links for itself when it starts.
$Portable = Join-Path $TargetDir "windows-package\$Pkg-$Version-windows-$Arch-portable"
Remove-Item -Recurse -Force $Portable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Portable | Out-Null
Copy-Item (Join-Path $Stage $Exe) $Portable
Copy-Item (Join-Path $Stage $Com) $Portable
foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Portable }
}
Copy-Item (Join-Path $Root 'packaging\README-desktop.md') (Join-Path $Portable 'README.md')
$Zip = Join-Path $Dist "$Pkg-$Version-windows-$Arch-portable.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Portable -DestinationPath $Zip

Get-Item $Msi, $Zip | Format-Table Name, Length
