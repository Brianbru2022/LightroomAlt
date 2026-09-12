$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$artifactDirectory = Join-Path $projectRoot "artifacts"
New-Item -ItemType Directory -Path $artifactDirectory -Force | Out-Null
$outputPath = Join-Path $artifactDirectory "m11-export-benchmark.txt"
Push-Location (Join-Path $projectRoot "src-tauri")
try {
    cargo test professional_export::tests::milestone_11_export_performance_checkpoint -- --ignored --nocapture 2>&1 | Tee-Object -FilePath $outputPath
} finally {
    Pop-Location
}
Write-Host "Milestone 11 export benchmark written to $outputPath"
