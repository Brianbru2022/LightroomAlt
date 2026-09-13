param(
    [string]$Python = "D:\AI Models\Keepframe\runtime\Scripts\python.exe",
    [int]$DenoiseSize = 256,
    [int]$SuperResolutionSize = 128,
    [switch]$CpuSmoke
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Python -PathType Leaf)) { throw "Provision the Keepframe runtime or pass -Python explicitly." }
$Arguments = @((Join-Path $PSScriptRoot "benchmark-ai-enhancement.py"), "--denoise-size", $DenoiseSize, "--sr-size", $SuperResolutionSize, "--warm-runs", 3)
if ($CpuSmoke) { $Arguments += "--cpu-smoke" }
& $Python @Arguments
if ($LASTEXITCODE -ne 0) { throw "AI enhancement benchmark failed." }
