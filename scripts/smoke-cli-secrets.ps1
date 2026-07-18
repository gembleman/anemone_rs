$ErrorActionPreference = 'Stop'

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("anemone-cli-secret-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
$exe = Join-Path $PSScriptRoot '..\target\i686-pc-windows-msvc\release\anemone_rs.exe'
$secret = 'cli-test-secret-7d1dc09d'
$oldLocalAppData = $env:LOCALAPPDATA
try {
    $env:LOCALAPPDATA = $root
    $setOutput = (($secret + "`n") | & $exe config set-secret translation.llm.api_key 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "set-secret failed: $setOutput" }
    if ($setOutput.Contains($secret)) { throw 'set-secret echoed the secret' }

    $showOutput = (& $exe config show 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "config show failed: $showOutput" }
    if ($showOutput.Contains($secret)) { throw 'config show exposed the secret' }
    if (-not $showOutput.Contains('***')) { throw 'config show did not include a redaction marker' }

    $getOutput = (& $exe config get translation.llm.api_key 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "config get failed: $getOutput" }
    if ($getOutput.Contains($secret)) { throw 'config get exposed the secret' }
    if (-not $getOutput.Contains('***')) { throw 'config get did not redact the secret' }
    Write-Host 'CLI secret redaction and stdin input smoke test passed.'
}
finally {
    $env:LOCALAPPDATA = $oldLocalAppData
    Remove-Item -LiteralPath $root -Recurse -Force
}
