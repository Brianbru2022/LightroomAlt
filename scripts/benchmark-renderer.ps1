$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    cargo test --manifest-path .\src-tauri\Cargo.toml milestone_10_renderer_performance_checkpoint -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw "Renderer benchmark failed with exit code $LASTEXITCODE" }
    cargo test --manifest-path .\src-tauri\Cargo.toml semantic_mask_performance_checkpoint -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw "Semantic-mask benchmark failed with exit code $LASTEXITCODE" }
}
finally {
    Pop-Location
}
