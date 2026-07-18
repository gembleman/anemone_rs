$ErrorActionPreference = 'Stop'

$manifest = Join-Path $PSScriptRoot '..\eztrans_dll\checksums.sha256'
foreach ($line in Get-Content -LiteralPath $manifest) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $parts = $line -split '\s+', 2
    $expected = $parts[0].ToLowerInvariant()
    $path = Join-Path (Join-Path $PSScriptRoot '..') $parts[1]
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "Asset checksum mismatch: $($parts[1])"
    }
}

$spec = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\SPEC.md')
$links = [regex]::Matches($spec, '`((?:src|eztrans_dll|\.cargo)/[^`]+)`')
foreach ($match in $links) {
    $relative = $match.Groups[1].Value.TrimEnd('/')
    $path = Join-Path (Join-Path $PSScriptRoot '..') $relative
    if (-not (Test-Path -LiteralPath $path)) {
        throw "SPEC.md references a missing path: $relative"
    }
}

$exe = Join-Path $PSScriptRoot '..\target\i686-pc-windows-msvc\release\anemone_rs.exe'
$bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $exe))
$peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
$signature = [Text.Encoding]::ASCII.GetString($bytes, $peOffset, 4)
if ($signature -ne "PE`0`0") { throw 'Invalid PE signature' }
$subsystem = [BitConverter]::ToUInt16($bytes, $peOffset + 0x5c)
if ($subsystem -ne 2) {
    throw "Expected Windows GUI subsystem (2), found $subsystem"
}
Write-Host 'Asset checksums and Windows GUI PE subsystem verified.'
