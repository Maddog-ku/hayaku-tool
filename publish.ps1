# 建置 HayakuBar（Rust 版）成單一 exe，輸出到 .\publish\rust\HayakuBar.exe（不需要安裝 .NET）
#   .\publish.ps1               建完刪掉 rust\target（約好幾 GB）
#   .\publish.ps1 -KeepTarget   保留 rust\target，下次建置比較快
# 需要 Rust（stable-x86_64-pc-windows-msvc）與 VS 2022 Build Tools；HayakuBar 正在執行時先結束它（exe 會被鎖住）。
param([switch]$KeepTarget)

$ErrorActionPreference = 'Stop'
$rust = Join-Path $PSScriptRoot 'rust'
$out = Join-Path $PSScriptRoot 'publish\rust'
cargo build --release -p hayakubar --locked --manifest-path (Join-Path $rust 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw "建置失敗（exit $LASTEXITCODE）" }
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $rust 'target\release\HayakuBar.exe') $out -Force
if (-not $KeepTarget) { Remove-Item -Recurse -Force (Join-Path $rust 'target') }
Write-Host "完成：$out\HayakuBar.exe"
