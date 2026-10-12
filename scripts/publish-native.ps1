[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$native = Join-Path $root 'native'
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    $cargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (-not (Test-Path $cargoPath -PathType Leaf)) { throw 'Rust 1.95+ is required; no tool is downloaded by this script.' }
} else {
    $cargoPath = $cargo.Source
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere -PathType Leaf)) { throw 'Visual Studio C++ Build Tools and Windows SDK are required.' }
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $installation) { throw 'No installed x64 Visual Studio C++ toolchain was found.' }
$setup = Join-Path $installation 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path $setup -PathType Leaf)) { throw 'The selected Visual Studio toolchain has no vcvars64.bat.' }
$installer = Split-Path -Parent $vswhere
$build = 'set "PATH={0};%PATH%" && call "{1}" >nul && cd /d "{2}" && "{3}" build --release --locked --offline --bin tomlook --target-dir target' -f $installer, $setup, $native, $cargoPath
& $env:ComSpec /d /c $build
if ($LASTEXITCODE -ne 0) { throw 'Native build failed. The current release was not changed.' }

$source = Join-Path $native 'target\release\tomlook.exe'
$release = Join-Path $root 'release'
$staging = Join-Path $root '.build\publish'
New-Item -ItemType Directory -Force -Path $release, $staging | Out-Null
$pending = Join-Path $staging ('Tomlook-{0}.exe' -f [guid]::NewGuid().ToString('N'))
$destination = Join-Path $release 'Tomlook.exe'
try {
    Copy-Item -LiteralPath $source -Destination $pending
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $pending -Algorithm SHA256).Hash) {
        throw 'Published executable does not match the build. The current release was not changed.'
    }
    if (Test-Path -LiteralPath $destination -PathType Leaf) {
        [System.IO.File]::Replace($pending, $destination, [NullString]::Value)
    } else {
        [System.IO.File]::Move($pending, $destination)
    }
} catch {
    throw "Publish failed; the previous release is retained. Exit a running release before publishing. $($_.Exception.Message)"
} finally {
    if (Test-Path -LiteralPath $pending -PathType Leaf) { Remove-Item -LiteralPath $pending }
}
Write-Output $destination
Get-FileHash -LiteralPath $destination -Algorithm SHA256
