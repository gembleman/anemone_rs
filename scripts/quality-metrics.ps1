# 리팩토링 품질 기준을 한 번에 재측정한다.
#
# 기준선(2026-09-01):
#   빌드/clippy/포맷 위반 0, 파일 LOC>500 0건, 순환 복잡도>15 0건,
#   인지 복잡도>20 0건, 프로덕션 패닉 경로 0건.
#
# 커버리지 기준은 "순수 로직 80% 이상"이다. Win32/COM을 직접 호출하는 코드는
# 실제 데스크톱·윈도우 핸들·D2D 디바이스가 있어야 실행되므로 단위 테스트로
# 덮지 않고, -Coverage 스위치를 줬을 때만 두 그룹을 나눠서 보고한다.
# (그 영역은 #[ignore] 표시된 Win32 스모크 테스트가 담당한다.)
#
# 사용법:
#   powershell -File scripts/quality-metrics.ps1              # 빠른 검사
#   powershell -File scripts/quality-metrics.ps1 -Coverage    # 커버리지 포함(느림)
param(
    [switch]$Coverage
)

$ErrorActionPreference = 'Stop'

# 한국어 출력이 깨지지 않도록 콘솔을 UTF-8로 맞춘다.
$previousEncoding = [Console]::OutputEncoding
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false

$repo = Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')
Push-Location -LiteralPath $repo

$failures = New-Object System.Collections.Generic.List[string]

# cargo 계열은 정상 동작 중에도 stderr로 진행 상황을 쓴다. $ErrorActionPreference
# 가 Stop이면 그걸 오류로 취급해 중단되므로, 네이티브 호출만 Continue로 감싼다.
function Invoke-Native {
    param([string]$Command, [string[]]$Arguments)

    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command @Arguments 2>&1 | ForEach-Object { "$_" }
    } finally {
        $ErrorActionPreference = $previous
    }
}

function Test-Metric {
    param([string]$Name, [int]$Actual, [int]$Limit)

    $status = if ($Actual -le $Limit) { 'OK  ' } else { 'FAIL' }
    Write-Host ("[{0}] {1}: {2}건 (허용 {3}건)" -f $status, $Name, $Actual, $Limit)
    if ($Actual -gt $Limit) {
        $failures.Add(('{0}: {1} > {2}' -f $Name, $Actual, $Limit))
    }
}

try {
    # --- 빌드 / lint / 포맷 -------------------------------------------------
    Write-Host '== 빌드 · lint · 포맷 =='
    $clippy = Invoke-Native cargo @('clippy', '--all-targets', '--all-features', '--message-format=short')
    $clippyHits = @($clippy | Select-String -Pattern '\.rs:\d+.*(warning|error)').Count
    Test-Metric -Name 'clippy 경고/에러' -Actual $clippyHits -Limit 0

    Invoke-Native cargo @('fmt', '--check') | Out-Null
    Test-Metric -Name 'rustfmt 위반' -Actual ([int]($LASTEXITCODE -ne 0)) -Limit 0

    # --- 파일 크기 ----------------------------------------------------------
    Write-Host ''
    Write-Host '== 유지보수성 =='
    $oversized = @(Get-ChildItem -Recurse -File -Include *.rs -Path src, tests |
            Where-Object { (Get-Content -LiteralPath $_.FullName).Count -gt 500 })
    foreach ($file in $oversized) {
        $count = (Get-Content -LiteralPath $file.FullName).Count
        Write-Host ("       초과: {0} ({1}줄)" -f (Resolve-Path -Relative $file.FullName), $count)
    }
    Test-Metric -Name '파일 LOC > 500' -Actual $oversized.Count -Limit 0

    # --- 복잡도 -------------------------------------------------------------
    # lizard(순환 복잡도)와 rust-code-analysis-cli(인지 복잡도). 없으면 건너뛴다.
    if (Get-Command lizard -ErrorAction SilentlyContinue) {
        # lizard는 모든 함수를 표로 찍은 뒤 마지막에 임계 초과분만 따로 모아준다.
        # 앞의 전체 표까지 세지 않도록 "Warnings" 헤더 이후만 읽는다.
        $lizard = @(Invoke-Native lizard @('src', '-l', 'rust', '-C', '15'))
        $warningStart = ($lizard | Select-String -Pattern '!!!! Warnings' | Select-Object -First 1).LineNumber
        $ccnHits = @()
        if ($warningStart) {
            $ccnHits = @($lizard[$warningStart..($lizard.Count - 1)] | Select-String -Pattern '@\d+-\d+@')
        }
        foreach ($hit in $ccnHits) {
            Write-Host ("       초과: {0}" -f $hit.Line.Trim())
        }
        Test-Metric -Name '순환 복잡도 > 15' -Actual $ccnHits.Count -Limit 0
    } else {
        Write-Host '[SKIP] lizard 미설치 - 순환 복잡도 검사 생략 (pip install lizard)'
    }

    if ((Get-Command rust-code-analysis-cli -ErrorAction SilentlyContinue) -and
        (Get-Command python -ErrorAction SilentlyContinue)) {
        $out = Join-Path $repo 'target/rca'
        Remove-Item -Recurse -Force -LiteralPath $out -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force -Path $out | Out-Null
        Invoke-Native rust-code-analysis-cli @('-m', '-p', 'src', '-O', 'json', '-o', $out) | Out-Null

        $helper = Join-Path $PSScriptRoot 'cognitive-complexity.py'
        $cognitive = @(Invoke-Native python @($helper, $out, '--limit', '20') |
                Where-Object { $_ -match '\S' })
        foreach ($hit in $cognitive) {
            Write-Host ("       초과: {0}" -f $hit)
        }
        Test-Metric -Name '인지 복잡도 > 20' -Actual $cognitive.Count -Limit 0
    } else {
        Write-Host '[SKIP] rust-code-analysis-cli 또는 python 미설치 - 인지 복잡도 검사 생략'
    }

    # --- Rust 안전성 --------------------------------------------------------
    Write-Host ''
    Write-Host '== Rust 안전성 =='
    if (Get-Command python -ErrorAction SilentlyContinue) {
        $helper = Join-Path $PSScriptRoot 'panic-paths.py'
        $panics = @(Invoke-Native python @($helper, '--path', 'src') |
                Where-Object { $_ -match '\S' })
        foreach ($hit in $panics) {
            Write-Host ("       {0}" -f $hit)
        }
        Test-Metric -Name '프로덕션 패닉 경로' -Actual $panics.Count -Limit 0
    } else {
        Write-Host '[SKIP] python 미설치 - 패닉 경로 검사 생략'
    }

    # --- 보안 ---------------------------------------------------------------
    Write-Host ''
    Write-Host '== 보안 =='
    if (Get-Command cargo-audit -ErrorAction SilentlyContinue) {
        Invoke-Native cargo @('audit') | Out-Null
        Test-Metric -Name 'cargo audit 취약점' -Actual ([int]($LASTEXITCODE -ne 0)) -Limit 0
    } else {
        Write-Host '[SKIP] cargo-audit 미설치 (cargo install cargo-audit)'
    }

    if (Get-Command cargo-machete -ErrorAction SilentlyContinue) {
        $machete = Invoke-Native cargo @('machete')
        $unused = [int](-not ($machete -match "didn't find any unused dependencies"))
        Test-Metric -Name '미사용 의존성' -Actual $unused -Limit 0
    } else {
        Write-Host '[SKIP] cargo-machete 미설치 (cargo install cargo-machete)'
    }

    # --- 커버리지 -----------------------------------------------------------
    if ($Coverage) {
        Write-Host ''
        Write-Host '== 테스트 커버리지 =='
        Invoke-Native cargo @('llvm-cov', '--no-report', '--workspace') | Out-Null
        $report = Invoke-Native cargo @('llvm-cov', 'report', '--summary-only')

        $pureLines = 0; $pureMissed = 0
        $winLines = 0; $winMissed = 0
        foreach ($line in $report) {
            if ($line -notmatch '^(\S+\.rs)\s+\d+\s+\d+\s+[\d.]+%\s+\d+\s+\d+\s+[\d.]+%\s+(\d+)\s+(\d+)\s+') { continue }
            $path = Join-Path 'src' ($Matches[1] -replace '\\', '/')
            $total = [int]$Matches[2]
            $missed = [int]$Matches[3]
            if (-not (Test-Path -LiteralPath $path)) { continue }

            # Win32/COM을 직접 부르는 파일은 실제 데스크톱 없이는 단위 테스트가 안 된다.
            if ((Get-Content -Raw -LiteralPath $path) -match 'use windows_sys::|use windows::|windows_sys::Win32|windows_core::') {
                $winLines += $total; $winMissed += $missed
            } else {
                $pureLines += $total; $pureMissed += $missed
            }
        }

        if ($pureLines -gt 0) {
            $percent = [math]::Round(($pureLines - $pureMissed) / $pureLines * 100, 2)
            $status = if ($percent -ge 80) { 'OK  ' } else { 'FAIL' }
            Write-Host ("[{0}] 순수 로직 (Win32 비의존): {1}% / {2}줄 (하한 80%)" -f $status, $percent, $pureLines)
            if ($percent -lt 80) {
                $failures.Add(('순수 로직 커버리지: {0}% < 80%' -f $percent))
            }
        }
        if ($winLines -gt 0) {
            $percent = [math]::Round(($winLines - $winMissed) / $winLines * 100, 2)
            Write-Host ("[INFO] Win32/COM 직접 호출: {0}% / {1}줄 - 기준 없음, Win32 스모크 테스트 담당" -f $percent, $winLines)
        }
    }

    # --- 결과 ---------------------------------------------------------------
    Write-Host ''
    if ($failures.Count -gt 0) {
        Write-Host ("품질 기준 미달 {0}건:" -f $failures.Count)
        foreach ($failure in $failures) { Write-Host ("  - {0}" -f $failure) }
        exit 1
    }
    Write-Host '모든 품질 기준을 충족합니다.'
} finally {
    Pop-Location
    [Console]::OutputEncoding = $previousEncoding
}
