$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$python = Join-Path $projectRoot "ai-worker\.test-venv\Scripts\python.exe"
$codexRuntime = Join-Path $env:USERPROFILE ".cache\codex-runtimes\codex-primary-runtime\dependencies"

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    $bundledNode = Join-Path $codexRuntime "node\bin\node.exe"
    if (Test-Path -LiteralPath $bundledNode) {
        $env:PATH = "$(Split-Path -Parent $bundledNode);$env:PATH"
    }
}

if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    $bundledPnpm = Join-Path $codexRuntime "bin\fallback\pnpm.cmd"
    if (Test-Path -LiteralPath $bundledPnpm) {
        $env:PATH = "$(Split-Path -Parent $bundledPnpm);$env:PATH"
    }
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

Push-Location $projectRoot
try {
    Invoke-Checked "pnpm" @("install", "--frozen-lockfile")
    Invoke-Checked "pnpm" @("test")
    Invoke-Checked "pnpm" @("build")
    Invoke-Checked "cargo" @("test", "--manifest-path", ".\src-tauri\Cargo.toml")
    Invoke-Checked "cargo" @("clippy", "--manifest-path", ".\src-tauri\Cargo.toml", "--all-targets", "--all-features", "--", "-D", "warnings")

    if (-not (Test-Path -LiteralPath $python)) {
        $pythonCommand = (Get-Command python -ErrorAction SilentlyContinue).Source
        if (-not $pythonCommand) {
            $pythonCommand = Join-Path $codexRuntime "python\python.exe"
        }
        if (-not (Test-Path -LiteralPath $pythonCommand)) {
            throw "Python 3 was not found. Install it or run this check from Codex Desktop."
        }
        Invoke-Checked $pythonCommand @("-m", "venv", (Join-Path $projectRoot "ai-worker\.test-venv"))
    }
    Invoke-Checked $python @("-m", "pip", "install", "--disable-pip-version-check", "-r", ".\ai-worker\requirements-test.txt")
    $env:PYTHONPATH = (Join-Path $projectRoot "ai-worker")
    Invoke-Checked $python @("-m", "unittest", "discover", "-s", ".\ai-worker\tests", "-v")
}
finally {
    Pop-Location
}
