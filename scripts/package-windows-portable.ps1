[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($Target -ne 'x86_64-pc-windows-msvc') {
    throw "The WinUSB portable bundle is currently gated to x86_64-pc-windows-msvc; got $Target."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$release = Join-Path $repoRoot "target\$Target\release"
$gui = Join-Path $release 'gem-imager-gui.exe'
$helper = Join-Path $release 'gem-winusb-helper.exe'
$runtime = Join-Path $repoRoot 'gem-winusb\native\x86_64-pc-windows-msvc\libwdi.dll'
$expectedRuntimeHash = 'C9F0AAA5A1B0A71B1740256168E3F0A870E979149765F0E2778B160377B69F27'

foreach ($required in @($gui, $helper, $runtime)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required portable component is missing: $required"
    }
}
$actualRuntimeHash = (Get-FileHash -LiteralPath $runtime -Algorithm SHA256).Hash
if ($actualRuntimeHash -ne $expectedRuntimeHash) {
    throw "libwdi.dll hash mismatch. Expected $expectedRuntimeHash; got $actualRuntimeHash"
}

$dist = Join-Path $repoRoot 'gem-imager-gui\dist'
$bundleName = "T3Gemstone_Imager_${Version}_x86_64_portable"
$bundle = Join-Path $dist $bundleName
$archive = Join-Path $dist "$bundleName.zip"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
if (Test-Path -LiteralPath $bundle) {
    Remove-Item -LiteralPath $bundle -Recurse -Force
}
if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}
New-Item -ItemType Directory -Path $bundle | Out-Null
Copy-Item -LiteralPath $gui -Destination (Join-Path $bundle 'gem-imager-gui.exe')
Copy-Item -LiteralPath $helper -Destination (Join-Path $bundle 'gem-winusb-helper.exe')
Copy-Item -LiteralPath $runtime -Destination (Join-Path $bundle 'libwdi.dll')
Copy-Item -LiteralPath (Join-Path $repoRoot 'gem-winusb\third_party\libwdi\COPYING-LGPL') -Destination $bundle
Copy-Item -LiteralPath (Join-Path $repoRoot 'gem-winusb\third_party\libwdi\Microsoft-WDF-License.rtf') -Destination $bundle
Compress-Archive -LiteralPath $bundle -DestinationPath $archive -CompressionLevel Optimal
Remove-Item -LiteralPath $bundle -Recurse -Force
Write-Output $archive
