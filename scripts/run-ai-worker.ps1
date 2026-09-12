param([int]$Port, [Parameter(Mandatory=$true)][string]$Token)
$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$DurablePython = "D:\AI Models\Keepframe\runtime\Scripts\python.exe"
$DevelopmentPython = Join-Path $ProjectRoot "ai-worker\.venv\Scripts\python.exe"
$Python = if (Test-Path -LiteralPath $DurablePython) { $DurablePython } else { $DevelopmentPython }
if (-not (Test-Path -LiteralPath $Python)) { throw "Keepframe analysis worker is not installed." }
$env:KEEPFRAME_MODEL_ROOT = "D:\AI Models\Keepframe"
$env:KEEPFRAME_WORKER_TOKEN = $Token
$env:HF_HOME = "D:\AI Models\Keepframe\huggingface"
$env:HF_HUB_OFFLINE = "1"
$env:TRANSFORMERS_OFFLINE = "1"
& $Python -m uvicorn keepframe_worker.main:app --app-dir (Join-Path $ProjectRoot "ai-worker") --host 127.0.0.1 --port $Port
