# 후킹 바이너리(lunahook DLL + x86 인젝터 헬퍼)를 빌드해 배치 레이아웃을 만든다.
#
# 산출물:
#   <target>/<triple>/<profile>/hook/x64/lunahook_rs64.dll
#   <target>/<triple>/<profile>/hook/x86/lunahook_rs32.dll
#   <target>/<triple>/<profile>/hook/x86/anemone_inject32.exe
#
# anemone.exe는 실행 파일 옆 hook/<arch>/ 폴더에서 로드한다(src/hook/mod.rs).
#
# 사용: pwsh scripts/build_hooks.ps1 [-Profile release|debug] [-TripleFilter x86_64|i686]
param(
    [ValidateSet("release", "debug")]
    [string]$Profile = "release",
    [ValidateSet("", "x86_64", "i686")]
    [string]$TripleFilter = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$lunahookDir = Join-Path $repoRoot "..\eztrans_scratch\lunahook_rs" | Resolve-Path

function Build-HookTargets {
    param([string]$Triple)

    $cargoProfileArgs = @()
    if ($Profile -eq "release") {
        $cargoProfileArgs += "--release"
    }

    Write-Host "=== [$Triple] lunahook_rs (cdylib) 빌드 ==="
    Push-Location $lunahookDir
    try {
        cargo build --target $Triple @cargoProfileArgs
        if ($LASTEXITCODE -ne 0) { throw "lunahook_rs 빌드 실패 ($Triple)" }
    } finally {
        Pop-Location
    }

    $lunahookDll = Join-Path $lunahookDir "target\$Triple\$Profile\lunahook_rs.dll"
    if (-not (Test-Path $lunahookDll)) {
        throw "cdylib 산출물이 없습니다: $lunahookDll"
    }

    $archFolder = if ($Triple.StartsWith("x86_64")) { "x64" } else { "x86" }
    # anemone은 x64 단일 빌드이고 실행 파일 옆 hook/<arch>/를 본다
    # (src/hook/mod.rs::hook_dir). 산출물 아키텍처와 무관하게 x64 호스트
    # 트리에 모아야 배치 레이아웃(x64 DLL과 x86 DLL+헬퍼 공존)이 완성된다.
    $hostTree = Join-Path $repoRoot "target\x86_64-pc-windows-msvc\$Profile"
    $destDir = Join-Path $hostTree "hook\$archFolder"
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null

    $dllName = if ($archFolder -eq "x64") { "lunahook_rs64.dll" } else { "lunahook_rs32.dll" }
    $dllDestination = Join-Path $destDir $dllName
    $needsCopy = -not (Test-Path -LiteralPath $dllDestination)
    if (-not $needsCopy) {
        $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $lunahookDll).Hash
        $destinationHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $dllDestination).Hash
        $needsCopy = $sourceHash -ne $destinationHash
    }
    if ($needsCopy) {
        try {
            Copy-Item -LiteralPath $lunahookDll -Destination $dllDestination -Force
        } catch {
            throw "후킹 DLL을 교체할 수 없습니다: $dllDestination`n이 DLL을 사용 중인 게임을 완전히 종료한 뒤 다시 실행해 주세요.`n$($_.Exception.Message)"
        }
        Write-Host "복사됨: $($lunahookDll) -> $dllDestination"
    } else {
        Write-Host "최신 상태: $dllDestination"
    }

    if ($archFolder -eq "x86") {
        Write-Host "=== [$Triple] anemone_inject32 빌드 ==="
        $inject32Manifest = Join-Path $repoRoot "tools\inject32\Cargo.toml"
        cargo build --manifest-path $inject32Manifest --target $Triple @cargoProfileArgs
        if ($LASTEXITCODE -ne 0) { throw "anemone_inject32 빌드 실패" }
        $helper = Join-Path $repoRoot "tools\inject32\target\$Triple\$Profile\anemone_inject32.exe"
        if (-not (Test-Path $helper)) {
            # 독립 crate라 자체 target 폴더에 쓴다. 예외적으로 상위 target에
            # 생기는 경우도 대비한다.
            $helper = Join-Path $repoRoot "target\$Triple\$Profile\anemone_inject32.exe"
        }
        if (-not (Test-Path $helper)) {
            throw "anemone_inject32.exe 산출물을 찾지 못했습니다"
        }
        Copy-Item -Force $helper (Join-Path $destDir "anemone_inject32.exe")
        Write-Host "복사됨: $($helper) -> $(Join-Path $destDir 'anemone_inject32.exe')"
    }
}

$targets = @()
if ($TripleFilter -eq "" -or $TripleFilter -eq "x86_64") {
    $targets += "x86_64-pc-windows-msvc"
}
if ($TripleFilter -eq "" -or $TripleFilter -eq "i686") {
    $targets += "i686-pc-windows-msvc"
}

foreach ($triple in $targets) {
    Build-HookTargets -Triple $triple
}

Write-Host "`n후킹 바이너리 빌드 완료."
