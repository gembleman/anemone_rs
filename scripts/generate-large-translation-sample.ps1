param(
    [int]$LineCount = 200000,
    [int]$UniqueLineCount = 10000
)

$ErrorActionPreference = 'Stop'
if ($LineCount -lt 1 -or $UniqueLineCount -lt 1 -or $UniqueLineCount -gt $LineCount) {
    throw 'LineCount와 UniqueLineCount는 1 이상이고 UniqueLineCount <= LineCount여야 합니다.'
}

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$largeOutputPath = Join-Path $repositoryRoot 'benchmark\large_japanese_translation_sample.txt'
$uniqueOutputPath = Join-Path $repositoryRoot 'benchmark\unique_japanese_translation_sample.txt'
$encoding = [System.Text.UTF8Encoding]::new($false)

function Write-TranslationSample {
    param(
        [string]$Path,
        [int]$Count,
        [int]$SentenceCount
    )

    $writer = [System.IO.StreamWriter]::new($Path, $false, $encoding, 1048576)
    try {
        for ($index = 0; $index -lt $Count; $index++) {
            $sentence = $index % $SentenceCount
            $writer.Write('第{0:D5}行です。' -f $sentence)
            if ($index + 1 -lt $Count) {
                $writer.Write("`n")
            }
        }
    }
    finally {
        $writer.Dispose()
    }
}

Write-TranslationSample -Path $largeOutputPath -Count $LineCount -SentenceCount $UniqueLineCount
Write-TranslationSample -Path $uniqueOutputPath -Count $UniqueLineCount -SentenceCount $UniqueLineCount

foreach ($sample in @(
    [PSCustomObject]@{ Path = $largeOutputPath; Lines = $LineCount; Label = 'repeated' },
    [PSCustomObject]@{ Path = $uniqueOutputPath; Lines = $UniqueLineCount; Label = 'unique' }
)) {
    $bytes = (Get-Item -LiteralPath $sample.Path).Length
    Write-Output "generated ($($sample.Label)): $($sample.Path) ($($sample.Lines) lines, $UniqueLineCount unique, $bytes bytes)"
}
