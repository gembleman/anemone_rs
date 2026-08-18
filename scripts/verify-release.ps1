$ErrorActionPreference = 'Stop'

# eztrans_dll/은 .gitignore 대상이라 CI 체크아웃에는 존재하지 않는다.
# 그곳에서 검증할 것은 아래 PE subsystem 검사뿐이므로, 매니페스트가 없으면
# 체크섬 검증만 건너뛴다. 있는데 내용이 틀린 경우는 여전히 실패시킨다.
$manifest = Join-Path $PSScriptRoot '..\eztrans_dll\checksums.sha256'
if (Test-Path -LiteralPath $manifest) {
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
    Write-Host 'Asset checksums verified.'
} else {
    Write-Host 'eztrans_dll/checksums.sha256 없음 - 체크섬 검증 건너뜀.'
}

$exe = Join-Path $PSScriptRoot '..\target\x86_64-pc-windows-msvc\release\anemone_rs.exe'
$bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $exe))
$peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
$signature = [Text.Encoding]::ASCII.GetString($bytes, $peOffset, 4)
if ($signature -ne "PE`0`0") { throw 'Invalid PE signature' }
$subsystem = [BitConverter]::ToUInt16($bytes, $peOffset + 0x5c)
if ($subsystem -ne 2) {
    throw "Expected Windows GUI subsystem (2), found $subsystem"
}
Write-Host 'Windows GUI PE subsystem verified.'
