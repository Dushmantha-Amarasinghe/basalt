# Puts ffmpeg into `lib/`, for converting video as it is watched.
#
# The host converts a film a device cannot play, on the machine's graphics
# where it can, and ffmpeg is what does it. It is not in the repository for
# the same reason libmpv is not: a binary this size in git history is a cost
# every clone pays for ever.
#
# Pinned to one release of gyan.dev's "essentials" build, which carries the
# hardware encoders and decoders the host uses (NVIDIA, Intel, AMD) and
# nothing it does not. Checked against the checksum gyan.dev publishes for
# it: this is a program that ends up inside the installer.
#
#   pwsh -File fetch-ffmpeg.ps1
#
# Safe to re-run: it skips ffmpeg when it is already here, unless -Force.

param([switch]$Force)

$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot 'lib'
New-Item -ItemType Directory -Force -Path $lib | Out-Null

$version = '9.0.2'
$name = "ffmpeg-$version-essentials_build"
$url = "https://github.com/GyanD/codexffmpeg/releases/download/$version/$name.zip"
# gyan.dev's own, from https://www.gyan.dev/ffmpeg/builds/packages/$name.zip.sha256
$sha = '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba'

$target = Join-Path $lib 'ffmpeg.exe'
if ((Test-Path $target) -and -not $Force) {
    Write-Host '  ffmpeg.exe already here'
    exit 0
}

Write-Host "fetching ffmpeg $version (essentials; ~110 MB)..."
$zip = Join-Path $env:TEMP "$name.zip"
if (-not (Test-Path $zip) -or ((Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() -ne $sha)) {
    Invoke-WebRequest -Uri $url -OutFile $zip
}
$got = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($got -ne $sha) {
    throw "ffmpeg checksum mismatch`n  expected $sha`n  got      $got"
}

$out = Join-Path $env:TEMP $name
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
Expand-Archive -Path $zip -DestinationPath $env:TEMP -Force
Copy-Item (Join-Path $out 'bin\ffmpeg.exe') $target -Force
# Its licence travels with it: GPL, version 3.
Copy-Item (Join-Path $out 'LICENSE') (Join-Path $lib 'ffmpeg-LICENSE.txt') -Force
Write-Host "  ffmpeg.exe is in $lib"
