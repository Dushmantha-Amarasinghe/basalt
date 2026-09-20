# Fetches the two libraries the player needs, into `lib/`.
#
# They are not in the repository because `libmpv-2.dll` is 96 MB, and a
# binary that size in git history is a cost every clone pays for ever. The
# checksum of the wrapper is verified against the one its author publishes;
# libmpv comes from zhongfly's builds, which is where mpv's own documentation
# points Windows users.
#
#   pwsh -File fetch-libmpv.ps1
#
# Safe to re-run: it skips anything already present unless -Force is given.

param([switch]$Force)

$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot 'lib'
New-Item -ItemType Directory -Force -Path $lib | Out-Null

$wrapperVersion = 'v0.1.1'
$wrapperUrl = "https://github.com/nini22P/libmpv-wrapper/releases/download/$wrapperVersion/libmpv-wrapper-windows-x86_64.zip"
# From that release's own sha256.txt. Checked rather than trusted: this is a
# binary that ends up inside the installer.
$wrapperSha = 'd2ff8b2edcd34d2968e544adaa915e5e5c48eb1a0995945005269c2af119a492'

function Need($name) {
    $path = Join-Path $lib $name
    if ((Test-Path $path) -and -not $Force) {
        Write-Host "  $name already here"
        return $false
    }
    return $true
}

if (Need 'libmpv-wrapper.dll') {
    Write-Host 'fetching libmpv-wrapper...'
    $zip = Join-Path $env:TEMP 'libmpv-wrapper.zip'
    Invoke-WebRequest -Uri $wrapperUrl -OutFile $zip
    $got = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($got -ne $wrapperSha) {
        throw "libmpv-wrapper checksum mismatch`n  expected $wrapperSha`n  got      $got"
    }
    $out = Join-Path $env:TEMP 'libmpv-wrapper'
    Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
    Expand-Archive -Path $zip -DestinationPath $out
    Get-ChildItem -Recurse -Path $out -Filter '*.dll' |
        ForEach-Object { Copy-Item $_.FullName $lib -Force }
    Write-Host '  ok'
}

if (Need 'libmpv-2.dll') {
    Write-Host 'fetching libmpv (the latest build; ~28 MB)...'
    # Resolved at run time rather than pinned: these builds are published
    # continuously and an old pin would rot into a 404.
    $release = Invoke-RestMethod 'https://api.github.com/repos/zhongfly/mpv-winbuild/releases/latest' `
        -Headers @{ 'User-Agent' = 'basalt-setup' }
    $asset = $release.assets |
        Where-Object { $_.name -like 'mpv-dev-lgpl-x86_64-*' -and $_.name -notlike '*-v3-*' } |
        Select-Object -First 1
    if (-not $asset) { throw 'no mpv-dev-lgpl build in the latest release' }

    $archive = Join-Path $env:TEMP $asset.name
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archive
    $seven = @(
        'C:\Program Files\7-Zip\7z.exe',
        'C:\Program Files (x86)\7-Zip\7z.exe'
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $seven) { throw 'needs 7-Zip to unpack the .7z (winget install 7zip.7zip)' }

    $out = Join-Path $env:TEMP 'mpv-dev'
    Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
    & $seven x -y "-o$out" $archive | Out-Null
    Copy-Item (Join-Path $out 'libmpv-2.dll') $lib -Force
    Write-Host "  ok ($($asset.name))"
}

Get-ChildItem $lib -Filter '*.dll' |
    ForEach-Object { '{0,-24} {1,8:N1} MB' -f $_.Name, ($_.Length / 1MB) }
