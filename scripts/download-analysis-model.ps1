param([string]$ModelRoot = "D:\AI Models\Keepframe")
$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$Python = Join-Path $ProjectRoot "ai-worker\.venv\Scripts\python.exe"
if (-not (Test-Path -LiteralPath $Python)) { throw "Set up the analysis worker first with scripts\setup-ai-worker.ps1." }
if (-not $ModelRoot.StartsWith("D:\AI Models", [System.StringComparison]::OrdinalIgnoreCase)) { throw "AI model assets must remain under D:\AI Models." }
Write-Host "Qwen3-VL-8B-Instruct requires approximately 17.5 GB plus download/cache overhead."
Write-Host "Destination: $ModelRoot\Qwen3-VL-8B-Instruct"
$Confirmation = Read-Host "Type DOWNLOAD to continue"
if ($Confirmation -cne "DOWNLOAD") { Write-Host "Download cancelled; Keepframe will use its deterministic fallback."; exit 0 }
New-Item -ItemType Directory -Force -Path $ModelRoot | Out-Null
$env:HF_HOME = Join-Path $ModelRoot "huggingface"
& $Python -c "from huggingface_hub import snapshot_download; snapshot_download('Qwen/Qwen3-VL-8B-Instruct', local_dir=r'$ModelRoot\Qwen3-VL-8B-Instruct')"
Write-Host "Analysis model installed."
