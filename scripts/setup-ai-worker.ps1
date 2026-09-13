param(
    [string]$Python = "python",
    [string]$RuntimeRoot = "D:\AI Models\Keepframe\runtime"
)
$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$WorkerRoot = Join-Path $ProjectRoot "ai-worker"
$Venv = $RuntimeRoot
if (-not $RuntimeRoot.StartsWith("D:\AI Models", [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "The Keepframe analysis runtime must remain under D:\AI Models."
}
Write-Host "Installing the Keepframe analysis runtime into $Venv. This may require several GB for PyTorch and Transformers."
Write-Host "Model weights are not downloaded by this script."
& $Python -m venv $Venv
if ($LASTEXITCODE -ne 0) { throw "Could not create the analysis runtime." }
$WorkerPython = Join-Path $Venv "Scripts\python.exe"
& $WorkerPython -m pip install --upgrade pip
if ($LASTEXITCODE -ne 0) { throw "Could not update pip in the analysis runtime." }
& $WorkerPython -m pip install -r (Join-Path $WorkerRoot "requirements.txt")
if ($LASTEXITCODE -ne 0) { throw "Could not install the analysis runtime dependencies." }
Write-Host "Local AI runtime ready. Models are separate explicit installs: 17.5 GB analysis, 900 MB intelligent masking, 72 MB AI Denoise, or 67 MB Super Resolution."
