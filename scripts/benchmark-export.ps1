$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$artifactDirectory = Join-Path $projectRoot "artifacts"
New-Item -ItemType Directory -Path $artifactDirectory -Force | Out-Null
$outputPath = Join-Path $artifactDirectory "m11-export-benchmark.txt"
Push-Location (Join-Path $projectRoot "src-tauri")
try {
    # Capture Cargo's ordinary stderr progress without Windows PowerShell
    # converting it into NativeCommandError records in the evidence file.
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = (Get-Command cargo).Source
    $startInfo.WorkingDirectory = (Get-Location).Path
    $startInfo.Arguments = 'test professional_export::tests::milestone_11_export_performance_checkpoint -- --ignored --nocapture'
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
    $evidence | Set-Content -LiteralPath $outputPath -Encoding utf8
    Write-Host $evidence
    if ($process.ExitCode -ne 0) { throw "Export benchmark failed with exit code $($process.ExitCode)" }
} finally {
    Pop-Location
}
Write-Host "Milestone 11 export benchmark written to $outputPath"
