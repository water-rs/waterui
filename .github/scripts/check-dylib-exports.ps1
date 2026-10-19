# Counts the PE export table of each image passed in — the exact DLLs the
# step's `cargo build --message-format=json-render-diagnostics` run reported
# — and fails when one exceeds the budget. The budget is 80% of the
# 65,535-entry PE export limit — headroom for churn between budget checks
# (#1615). Prints every DLL's name and count.
param(
    [Parameter(Mandatory)]
    [string[]]$Dll
)
$ErrorActionPreference = 'Stop'
$budget = 52428

if (-not $Dll) {
    Write-Error 'no DLL paths given — pass the filenames from the build''s compiler-artifact messages'
}

$sysroot = (& rustc --print sysroot).Trim()
$readobj = Join-Path $sysroot 'lib\rustlib\x86_64-pc-windows-msvc\bin\llvm-readobj.exe'
if (-not (Test-Path $readobj)) {
    & rustup component add llvm-tools | Out-Null
}
if (-not (Test-Path $readobj)) {
    Write-Error "llvm-readobj not found at $readobj (install the llvm-tools component)"
}

$failed = $false
foreach ($path in ($Dll | Sort-Object)) {
    $name = Split-Path $path -Leaf
    if (-not (Test-Path $path)) {
        Write-Error "dylib not found at $path — the path must come from the build just run"
    }
    $exports = & $readobj --coff-exports $path
    if ($LASTEXITCODE -ne 0) {
        Write-Error "llvm-readobj failed on $name (exit $LASTEXITCODE)"
    }
    $count = ($exports | Select-String 'Name:').Count
    if ($count -eq 0) {
        Write-Error "$name reports zero exports — a dylib image always has some"
    }
    $status = 'ok'
    if ($count -gt $budget) {
        $status = 'OVER BUDGET'
        $failed = $true
    }
    Write-Host "$count`t$name`t$status"
}
if ($failed) {
    exit 1
}
