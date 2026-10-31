[CmdletBinding()]
param(
    # Publish the staged archive as an annotated tag and GitHub Release.
    [switch]$Publish,
    # Reuse the existing native\target\release\tomlook.exe instead of rebuilding it.
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$native = Join-Path $root 'native'

$manifest = Get-Content -LiteralPath (Join-Path $native 'Cargo.toml') -Raw
if ($manifest -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw 'native\Cargo.toml has no package version.' }
$version = $Matches[1]
$tag = "v$version"
$notes = Join-Path $root "docs\releases\$tag.md"
if (-not (Test-Path -LiteralPath $notes -PathType Leaf)) { throw "Release notes $notes are missing." }

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = if ($cargo) { $cargo.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path -LiteralPath $cargoPath -PathType Leaf)) { throw 'Rust 1.95+ is required; no tool is downloaded by this script.' }

if ($Publish) {
    if (git -C $root status --porcelain) { throw 'Commit all changes before publishing a release.' }
    if (git -C $root tag -l $tag) { throw "Tag $tag already exists." }
    git -C $root fetch --quiet origin
    $head = git -C $root rev-parse HEAD
    $remote = git -C $root rev-parse '@{u}'
    if ($head -ne $remote) { throw 'Push the release commit before publishing.' }
}

if (-not $SkipBuild) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere -PathType Leaf)) { throw 'Visual Studio C++ Build Tools and Windows SDK are required.' }
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $setup = Join-Path $installation 'VC\Auxiliary\Build\vcvars64.bat'
    if (-not (Test-Path $setup -PathType Leaf)) { throw 'No installed x64 Visual Studio C++ toolchain was found.' }
    $build = 'set "PATH={0};%PATH%" && call "{1}" >nul && cd /d "{2}" && "{3}" build --release --locked --offline --bin tomlook --target-dir target' -f (Split-Path -Parent $vswhere), $setup, $native, $cargoPath
    & $env:ComSpec /d /c $build
    if ($LASTEXITCODE -ne 0) { throw 'Native release build failed.' }
}
# Package the build output directly so a running release\Tomlook.exe is never touched.
$exe = Join-Path $native 'target\release\tomlook.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'native\target\release\tomlook.exe is missing.' }
$built = (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion
if ($built -ne $version) { throw "The built executable reports version $built, expected $version." }

function Get-NoticeId([string]$text) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($text)
    ([BitConverter]::ToString([Security.Cryptography.SHA256]::HashData($bytes)) -replace '-', '').ToLowerInvariant().Substring(0, 16)
}

$metadata = & $cargoPath metadata --format-version 1 --locked --offline --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $native 'Cargo.toml') | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
$members = @($metadata.workspace_members)
$licensePattern = '(?i)^(licen[cs]e|copying|notices?|unlicense|copyright)([-_.].*)?$|licen[cs]e\.txt$|^(ofl|ufl)\.txt$'
$texts = [ordered]@{}
$entries = New-Object System.Collections.Generic.List[string]
foreach ($package in ($metadata.packages | Where-Object { $members -notcontains $_.id } | Sort-Object name, version)) {
    $dir = Split-Path -Parent $package.manifest_path
    $files = @(Get-ChildItem -LiteralPath $dir -File -Recurse | Where-Object { $_.Name -match $licensePattern -and $_.Extension -ne '.rs' -and $_.FullName -notmatch '\\(target|\.git)\\' })
    if ($package.license_file) { $files += Get-Item -LiteralPath (Join-Path $dir $package.license_file) -ErrorAction SilentlyContinue }
    $block = @("$($package.name) $($package.version)")
    $block += 'License expression: ' + $(if ($package.license) { $package.license } else { 'see license file' })
    if ($package.source -like 'registry+*crates.io*') { $block += "Source package: https://crates.io/crates/$($package.name)/$($package.version)" }
    elseif ($package.source) { $block += "Source package: $($package.source)" }
    if ($package.repository) { $block += "Repository: $($package.repository)" }
    foreach ($file in ($files | Sort-Object FullName -Unique)) {
        $text = ([IO.File]::ReadAllText($file.FullName) -replace "`r`n", "`n").Trim()
        if (-not $text) { continue }
        $id = Get-NoticeId $text
        if (-not $texts.Contains($id)) { $texts[$id] = $text }
        $block += "$($file.FullName.Substring($dir.Length + 1)): notice $id"
    }
    $entries.Add(($block -join "`n"))
}

$header = "Tomlook $tag - third-party notices`n`nThis inventory covers the locked Windows dependency resolution, including build/test tooling; it does not imply every listed package is linked into the executable. Original licenses and notices follow. No proprietary Copilot runtime is bundled."
$body = $header + "`n`n`n" + ($entries -join "`n`n`n") + "`n`n`n" + (($texts.Keys | ForEach-Object { "===== Notice $_ =====`n`n$($texts[$_])" }) -join "`n`n`n") + "`n"

$stage = Join-Path $root ".build\releases\$tag"
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
$content = Join-Path $stage 'content'
New-Item -ItemType Directory -Force -Path $content | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $content 'Tomlook.exe')
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $content 'LICENSE')
[IO.File]::WriteAllText((Join-Path $content 'THIRD_PARTY_NOTICES.txt'), $body, (New-Object Text.UTF8Encoding $false))
$zip = Join-Path $stage "Tomlook-$tag-win-x64.zip"
Compress-Archive -Path (Join-Path $content '*') -DestinationPath $zip -CompressionLevel Optimal
$sums = @(
    '{0}  Tomlook.exe' -f (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    '{0}  {1}' -f (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant(), (Split-Path -Leaf $zip)
)
$sumsPath = Join-Path $stage 'SHA256SUMS.txt'
[IO.File]::WriteAllText($sumsPath, ($sums -join "`n") + "`n", (New-Object Text.UTF8Encoding $false))
Remove-Item -LiteralPath $content -Recurse -Force
Write-Output "Staged $tag in $stage ($($entries.Count) packages, $($texts.Count) notices)."

if ($Publish) {
    git -C $root tag -a $tag -m "Tomlook $tag"
    git -C $root push --quiet origin $tag
    gh release create $tag $zip $sumsPath --repo tkubica12/ai-outlook --verify-tag --title "Tomlook $tag" --notes-file $notes
    if ($LASTEXITCODE -ne 0) { throw "GitHub Release creation failed; tag $tag was pushed." }
    Write-Output "Published https://github.com/tkubica12/ai-outlook/releases/tag/$tag"
}
