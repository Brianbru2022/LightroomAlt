$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$artifactDirectory = Join-Path $projectRoot "artifacts"
$artifact = Join-Path $artifactDirectory "m15-advanced-develop-benchmark.txt"
New-Item -ItemType Directory -Path $artifactDirectory -Force | Out-Null

function Invoke-CargoBenchmark {
    param(
        [Parameter(Mandatory = $true)][string]$Arguments,
        [switch]$Append
    )
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = (Get-Command cargo).Source
    $startInfo.WorkingDirectory = $projectRoot
    $startInfo.Arguments = $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    [void]$process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $evidence = ($stderr.Result.TrimEnd(), $stdout.Result.TrimEnd() | Where-Object { $_ }) -join [Environment]::NewLine
    if ($Append) { $evidence | Add-Content -LiteralPath $artifact -Encoding utf8 }
    else { $evidence | Set-Content -LiteralPath $artifact -Encoding utf8 }
    Write-Host $evidence
    if ($process.ExitCode -ne 0) { throw "Cargo benchmark failed with exit code $($process.ExitCode)" }
}

Push-Location $projectRoot
try {
    Invoke-CargoBenchmark 'test --manifest-path .\src-tauri\Cargo.toml advanced_develop_performance_checkpoint -- --ignored --nocapture'
    Invoke-CargoBenchmark 'test --manifest-path .\src-tauri\Cargo.toml milestone_10_renderer_performance_checkpoint -- --ignored --nocapture' -Append
    Write-Output "Milestone 15 benchmark written to $artifact"
}
finally {
    Pop-Location
}
