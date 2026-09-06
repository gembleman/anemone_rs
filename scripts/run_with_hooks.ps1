# Cargo가 호스트 실행 파일을 빌드한 뒤 최신 후킹 바이너리를 배치하고 실행한다.
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Executable,

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$ExecutableArgs
)

$ErrorActionPreference = "Stop"

# Cargo의 target runner는 test/example 실행에도 적용된다. 후킹 바이너리는
# 실제 anemone 애플리케이션을 실행할 때만 준비한다.
if ([System.IO.Path]::GetFileName($Executable) -ne "anemone_rs.exe") {
    & $Executable @ExecutableArgs
    exit $LASTEXITCODE
}

$profile = $null
$path = Split-Path -Parent $Executable
while ($path) {
    $leaf = Split-Path -Leaf $path
    if ($leaf -eq "release" -or $leaf -eq "debug") {
        $profile = $leaf
        break
    }
    $parent = Split-Path -Parent $path
    if ($parent -eq $path) {
        break
    }
    $path = $parent
}

if (-not $profile) {
    throw "실행 파일 경로에서 Cargo 프로필을 확인할 수 없습니다: $Executable"
}

& (Join-Path $PSScriptRoot "build_hooks.ps1") -Profile $profile
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

& $Executable @ExecutableArgs
exit $LASTEXITCODE
