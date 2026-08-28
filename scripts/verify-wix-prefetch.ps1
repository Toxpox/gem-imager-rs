#!/usr/bin/env pwsh
# Exercise the WIX prefetch step from .github/workflows/release.yml without a Windows runner.
#
# The Windows release once failed with "tls connection init failed (os error 10054)" because
# cargo-packager downloads the WIX3 toolset itself, with no retry. The workflow now prefetches
# and validates the toolset instead. That step only ever runs on a tagged release, so a mistake
# in it stays invisible until a release breaks: this harness runs it on demand instead.
#
# The step body is read straight out of the workflow, so the tested code cannot drift from the
# shipped code. Only the three network/filesystem cmdlets are stubbed.
#
# Usage:
#   pwsh scripts/verify-wix-prefetch.ps1
#   docker run --rm -v "$PWD:/w" -w /w mcr.microsoft.com/powershell:latest \
#     pwsh -File scripts/verify-wix-prefetch.ps1

param(
    [string]$Workflow = (Join-Path $PSScriptRoot '..' '.github/workflows/release.yml'),
    [string]$StepName = 'Prefetch and verify WIX toolset'
)

$ErrorActionPreference = 'Stop'

# Pull the step body out of the workflow. Parsed by hand rather than with a YAML module so the
# harness has no dependency beyond PowerShell itself.
function Get-StepScript {
    param([string]$Path, [string]$Name)

    $lines = Get-Content $Path
    $start = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match "^\s*-?\s*name:\s*$([regex]::Escape($Name))\s*$") { $start = $i; break }
    }
    if ($start -lt 0) { throw "step '$Name' not found in $Path" }

    $runAt = -1
    for ($i = $start; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^\s*run:\s*\|') { $runAt = $i; break }
        # A new step began before any `run:` block.
        if ($i -gt $start -and $lines[$i] -match '^\s*-\s*name:') { break }
    }
    if ($runAt -lt 0) { throw "step '$Name' has no block `run:` scalar" }

    $indent = ($lines[$runAt] -replace '\S.*$').Length
    $body = @()
    for ($i = $runAt + 1; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line.Trim() -eq '') { $body += ''; continue }
        $cur = ($line -replace '\S.*$').Length
        if ($cur -le $indent) { break }
        $body += $line.Substring($indent + 2)
    }
    return ($body -join "`n").TrimEnd()
}

$script = Get-StepScript -Path $Workflow -Name $StepName
Write-Host "Loaded '$StepName' from $Workflow ($($script.Length) bytes)`n"

# Replaces only the cmdlets that touch the network or unpack an archive. $global:Scenario picks
# the failure being simulated; everything else runs the real logic.
$prelude = @'
$global:Attempts = 0
function Invoke-WebRequest {
  param($Uri, $OutFile, [switch]$UseBasicParsing, $TimeoutSec)
  $global:Attempts++
  if ($global:Scenario -eq 'network-flaky' -and $global:Attempts -lt 3) {
    throw "tls connection init failed (os error 10054)"
  }
  if ($global:Scenario -eq 'network-dead') { throw "tls connection init failed (os error 10054)" }
  Set-Content -Path $OutFile -Value 'stub-zip'
}
function Get-FileHash {
  param($Path, $Algorithm)
  if ($global:Scenario -eq 'bad-checksum') { return [pscustomobject]@{ Hash = 'DEADBEEF' } }
  return [pscustomobject]@{ Hash = '2C1888D5D1DBA377FC7FA14444CF556963747FF9A0A289A3599CF09DA03B9E2E' }
}
function Expand-Archive {
  param($Path, $DestinationPath, [switch]$Force)
  New-Item -ItemType Directory -Force -Path $DestinationPath | Out-Null
  if ($global:Scenario -eq 'truncated-archive') {
    Set-Content -Path (Join-Path $DestinationPath 'candle.exe') -Value 'x'
    return
  }
  foreach ($f in @('candle.exe','candle.exe.config','darice.cub','light.exe','light.exe.config',
                   'wconsole.dll','winterop.dll','wix.dll','WixUIExtension.dll','WixUtilExtension.dll')) {
    Set-Content -Path (Join-Path $DestinationPath $f) -Value 'x'
  }
}
'@

# Mirrors WIX_REQUIRED_FILES in cargo-packager: the set it checks before deciding to refetch.
$all = @('candle.exe','candle.exe.config','darice.cub','light.exe','light.exe.config',
         'wconsole.dll','winterop.dll','wix.dll','WixUIExtension.dll','WixUtilExtension.dll')

$root = Join-Path ([System.IO.Path]::GetTempPath()) "wix-verify-$PID"
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue

function Invoke-Scenario {
    param([string]$Name, [string[]]$Seed)

    $local = Join-Path $root "$Name/local"
    $temp = Join-Path $root "$Name/temp"
    New-Item -ItemType Directory -Force -Path $local, $temp | Out-Null
    $env:LOCALAPPDATA = $local
    $env:RUNNER_TEMP = $temp

    if ($Seed) {
        $d = Join-Path $local '.cargo-packager/WixTools'
        New-Item -ItemType Directory -Force -Path $d | Out-Null
        foreach ($f in $Seed) { Set-Content -Path (Join-Path $d $f) -Value 'x' }
    }

    $body = "`$global:Scenario = '$Name'`n" + $prelude + "`n" + $script
    $out = pwsh -NoProfile -Command $body 2>&1 | Out-String
    [pscustomobject]@{
        Name   = $Name
        Exit   = $LASTEXITCODE
        Output = $out.Trim()
        Dir    = (Join-Path $local '.cargo-packager/WixTools')
    }
}

# Each case is a way the step is actually reached on a runner, plus what must be true afterwards.
$cases = @(
    @{ Name = 'cache-hit-complete'; Seed = $all
       Exit = 0; Must = 'restored from cache'; MustNot = 'attempt 1 failed'
       Why  = 'a warm cache must be trusted without re-downloading' }

    @{ Name = 'cache-hit-partial'; Seed = @('candle.exe', 'light.exe')
       Exit = 0; Must = 'incomplete, missing'; MustNot = $null
       Why  = 'a partial cache still reports cache-hit, so the step must detect and refetch' }

    @{ Name = 'cache-miss'; Seed = $null
       Exit = 0; Must = 'ready at'; MustNot = $null
       Why  = 'the cold path must download and unpack' }

    @{ Name = 'network-flaky'; Seed = $null
       Exit = 0; Must = 'ready at'; MustNot = $null
       Why  = 'the original release failure: a dropped TLS connection must be retried' }

    @{ Name = 'network-dead'; Seed = $null
       Exit = 1; Must = 'attempt 5 failed'; MustNot = 'ready at'
       Why  = 'a persistent outage must fail the job rather than hang or pass' }

    @{ Name = 'bad-checksum'; Seed = $null
       Exit = 1; Must = 'checksum mismatch'; MustNot = 'ready at'
       Why  = 'a corrupted or substituted archive must never be published to the cache' }

    @{ Name = 'truncated-archive'; Seed = $null
       Exit = 1; Must = 'did not contain'; MustNot = 'ready at'
       Why  = 'an archive missing tools must not be reported as ready' }
)

$failed = 0
foreach ($case in $cases) {
    $r = Invoke-Scenario -Name $case.Name -Seed $case.Seed
    $why = @()

    if ($r.Exit -ne $case.Exit) { $why += "exit $($r.Exit), expected $($case.Exit)" }
    if ($case.Must -and $r.Output -notmatch [regex]::Escape($case.Must)) {
        $why += "output missing '$($case.Must)'"
    }
    if ($case.MustNot -and $r.Output -match [regex]::Escape($case.MustNot)) {
        $why += "output contains '$($case.MustNot)'"
    }
    # Whatever the path taken, a zero exit has to leave a toolset cargo-packager will accept.
    if ($case.Exit -eq 0) {
        $missing = $all | Where-Object { -not (Test-Path (Join-Path $r.Dir $_)) }
        if ($missing) { $why += "left an incomplete toolset, missing: $($missing -join ', ')" }
    }

    if ($why.Count -eq 0) {
        Write-Host "  PASS  $($case.Name) - $($case.Why)"
    } else {
        $failed++
        Write-Host "  FAIL  $($case.Name) - $($case.Why)"
        foreach ($w in $why) { Write-Host "        $w" }
        Write-Host ($r.Output -split "`n" | ForEach-Object { "        | $_" }) -Separator "`n"
    }
}

Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue

Write-Host ''
if ($failed -gt 0) {
    Write-Host "$failed of $($cases.Count) scenarios failed"
    exit 1
}
Write-Host "all $($cases.Count) scenarios passed"
