$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Push-Location $Root
try {
    cargo install --path $Root --force
    $Bridge = Get-Command codex-o-pet-bridge -ErrorAction Stop
    Write-Host "Bridge installed at $($Bridge.Source)"
} finally {
    Pop-Location
}
