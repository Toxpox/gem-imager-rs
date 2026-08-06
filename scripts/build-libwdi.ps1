[CmdletBinding()]
param(
    [string]$BuildRoot,
    [switch]$ReplacePinnedBinary
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $BuildRoot) {
    $BuildRoot = Join-Path $repoRoot 'target\libwdi-build'
}
$BuildRoot = [System.IO.Path]::GetFullPath($BuildRoot)
$sourceArchive = Join-Path $repoRoot 'gem-winusb\third_party\libwdi\libwdi-v1.5.1.zip'
$expectedSourceHash = 'D74D27FDDBF5546C6A22A00FB67F9FC61A60B4AD9A7E974E9875E9CEE39BFAC7'
$expectedWdfHash = '29314207814CE9D5D73695F7E9239539CF37C79E750B9D5EA5A5EF5487A583D6'
$pinnedDllHash = 'C9F0AAA5A1B0A71B1740256168E3F0A870E979149765F0E2778B160377B69F27'
$wdfUrl = 'https://download.microsoft.com/download/0/5/F/05FD6919-6250-425B-86ED-9B095E54065A/wdfcoinstaller.msi'

function Assert-Sha256([string]$Path, [string]$Expected) {
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if ($actual -ne $Expected) {
        throw "SHA-256 mismatch for $Path. Expected $Expected; got $actual"
    }
}

function Invoke-MsBuild([string]$Project, [string]$Platform, [string]$SolutionDir) {
    & $script:msbuild $Project '/m' '/t:Build' '/p:Configuration=Release' "/p:Platform=$Platform" "/p:SolutionDir=$SolutionDir\"
    if ($LASTEXITCODE -ne 0) {
        throw "MSBuild failed for $Project ($Platform) with exit code $LASTEXITCODE"
    }
}

Assert-Sha256 $sourceArchive $expectedSourceHash
New-Item -ItemType Directory -Force -Path $BuildRoot | Out-Null

$sourceRoot = Join-Path $BuildRoot 'source'
if (Test-Path -LiteralPath $sourceRoot) {
    Remove-Item -LiteralPath $sourceRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $sourceRoot | Out-Null
Expand-Archive -LiteralPath $sourceArchive -DestinationPath $sourceRoot
$source = Get-ChildItem -LiteralPath $sourceRoot -Directory | Select-Object -First 1 -ExpandProperty FullName
if (-not $source) {
    throw 'The libwdi source archive did not contain a root directory.'
}

$wdfMsi = Join-Path $BuildRoot 'wdfcoinstaller.msi'
if (-not (Test-Path -LiteralPath $wdfMsi)) {
    Invoke-WebRequest -Uri $wdfUrl -OutFile $wdfMsi
}
Assert-Sha256 $wdfMsi $expectedWdfHash

$wdfRoot = Join-Path $BuildRoot 'wdf'
if (Test-Path -LiteralPath $wdfRoot) {
    Remove-Item -LiteralPath $wdfRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $wdfRoot | Out-Null
$msi = Start-Process -FilePath 'msiexec.exe' -ArgumentList @(
    '/a', "`"$wdfMsi`"", '/qn', "TARGETDIR=`"$wdfRoot`""
) -Wait -PassThru -WindowStyle Hidden
if ($msi.ExitCode -ne 0) {
    throw "Administrative extraction of the WDF package failed with exit code $($msi.ExitCode)."
}

$configPath = Join-Path $source 'msvc\config.h'
$config = Get-Content -LiteralPath $configPath -Raw
$wdfForC = (Join-Path $wdfRoot 'Windows Kits\8.0').Replace('\', '/')
$config = $config.Replace(
    '#define WDK_DIR "C:/Program Files (x86)/Windows Kits/8.0"',
    "#define WDK_DIR `"$wdfForC`""
)
$config = $config.Replace('#define LIBUSB0_DIR "D:/libusb-win32"', '// #define LIBUSB0_DIR disabled: WinUSB-only Gem Imager build')
$config = $config.Replace('#define LIBUSBK_DIR "D:/libusbK/bin"', '// #define LIBUSBK_DIR disabled: WinUSB-only Gem Imager build')
$config = $config.Replace('#define OPT_ARM', '// #define OPT_ARM disabled: x64 MVP')
Set-Content -LiteralPath $configPath -Value $config -Encoding UTF8

$projectPath = Join-Path $source 'libwdi\.msvc\libwdi_dll.vcxproj'
$project = Get-Content -LiteralPath $projectPath -Raw
$project = [regex]::Replace(
    $project,
    '(?s)\s*<ItemGroup>\s*<ProjectReference Include="embedder\.vcxproj">.*?</ItemGroup>',
    ''
)
Set-Content -LiteralPath $projectPath -Value $project -Encoding UTF8

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) {
    throw 'vswhere.exe was not found. Install Visual Studio with the Desktop development with C++ workload.'
}
$installPaths = & $vswhere -all -products '*' -requires Microsoft.Component.MSBuild -property installationPath
$installPath = $installPaths | Where-Object {
    Test-Path (Join-Path $_ 'MSBuild\Microsoft\VC\v170\Platforms\Win32\PlatformToolsets\v143\Toolset.props')
} | Select-Object -First 1
if (-not $installPath) {
    throw 'Visual Studio with the v143 x86/x64 C++ toolset was not found.'
}
$script:msbuild = Join-Path $installPath 'MSBuild\Current\Bin\MSBuild.exe'
$projectDir = Join-Path $source 'libwdi\.msvc'

Invoke-MsBuild (Join-Path $projectDir 'embedder.vcxproj') 'Win32' $source
Invoke-MsBuild (Join-Path $projectDir 'installer_x86.vcxproj') 'Win32' $source
Invoke-MsBuild (Join-Path $projectDir 'installer_x64.vcxproj') 'x64' $source
Invoke-MsBuild (Join-Path $projectDir 'libwdi_dll.vcxproj') 'x64' $source

$builtDll = Join-Path $source 'x64\Release\dll\libwdi.dll'
if (-not (Test-Path -LiteralPath $builtDll)) {
    throw "Expected output was not produced: $builtDll"
}
$builtHash = (Get-FileHash -LiteralPath $builtDll -Algorithm SHA256).Hash
Write-Output "Built libwdi.dll: $builtDll"
Write-Output "SHA-256: $builtHash"
if ($builtHash -ne $pinnedDllHash) {
    Write-Warning 'The build differs from the pinned reviewed DLL. Review exports/toolchain/output before changing the runtime allowlist.'
}

if ($ReplacePinnedBinary) {
    $destination = Join-Path $repoRoot 'gem-winusb\native\x86_64-pc-windows-msvc\libwdi.dll'
    Copy-Item -LiteralPath $builtDll -Destination $destination -Force
    Write-Output "Replaced pinned runtime: $destination"
}
