# 發佈 HayakuBar 成單一 exe，輸出到 .\publish
#   .\publish.ps1                 需要電腦有 .NET 10 Desktop Runtime（檔案小）
#   .\publish.ps1 -SelfContained  不需要安裝 .NET（檔案約 70MB，適合給同事）
param([switch]$SelfContained)

$ErrorActionPreference = 'Stop'
$out = Join-Path $PSScriptRoot 'publish'
dotnet publish (Join-Path $PSScriptRoot 'src\HayakuBar\HayakuBar.csproj') `
    -c Release -r win-x64 `
    --self-contained:$($SelfContained.IsPresent) `
    -p:PublishSingleFile=true `
    -p:IncludeNativeLibrariesForSelfExtract=true `
    -p:DebugType=none `
    -o $out
Write-Host "完成：$out\HayakuBar.exe"
