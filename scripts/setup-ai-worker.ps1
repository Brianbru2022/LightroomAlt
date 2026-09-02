param([string]$Python = "python")
$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$WorkerRoot = Join-Path $ProjectRoot "ai-worker"
$Venv = Join-Path $WorkerRoot ".venv"
Write-Host "Installing the Keepframe analysis runtime into $Venv."
Write-Host "Model weights are not downloaded by this script."
& $Python -m venv $Venv
$WorkerPython = Join-Path $Venv "Scripts\python.exe"
& $WorkerPython -m pip install --upgrade pip
& $WorkerPython -m pip install -r (Join-Path $WorkerRoot "requirements.txt")
Write-Host "Analysis runtime ready. Run scripts\download-analysis-model.ps1 separately for the 17.5 GB model."
