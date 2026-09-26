# Puts libmpv into `lib/`, for the video thumbnails the host makes.
#
# The same DLL the client plays with, so it is taken from the client's `lib/`
# when that is already there, and fetched through the client's own script
# when it is not — one place decides which build and checks what it gets.
#
#   pwsh -File fetch-libmpv.ps1

$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot 'lib'
$client = Join-Path $PSScriptRoot '..\..\client\src-tauri'
$source = Join-Path $client 'lib\libmpv-2.dll'

if (-not (Test-Path $source)) {
    & (Join-Path $client 'fetch-libmpv.ps1')
}
New-Item -ItemType Directory -Force -Path $lib | Out-Null
Copy-Item $source (Join-Path $lib 'libmpv-2.dll') -Force
Write-Host "libmpv-2.dll is in $lib"
