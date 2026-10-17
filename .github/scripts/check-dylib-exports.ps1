# Counts the PE export table of every waterui_dylib*.dll produced under
# target/debug/deps and fails when one exceeds the budget. The budget is 80%
# of the 65,535-entry PE export limit — headroom for churn between budget
# checks (#1615). Prints every DLL's name and count.
$ErrorActionPreference = 'Stop'
$budget = 52428

$sysroot = (& rustc --print sysroot).Trim()
$readobj = Join-Path $sysroot 'lib\rustlib\x86_64-pc-windows-msvc\bin\llvm-readobj.exe'
if (-not (Test-Path $readobj)) {
    & rustup component add llvm-tools | Out-Null
}
if (-not (Test-Path $readobj)) {
    Write-Error "llvm-readobj not found at $readobj (install the llvm-tools component)"
}

$dlls = Get-ChildItem 'target\debug\deps\waterui_dylib*.dll' -File
if (-not $dlls) {
    Write-Error 'no waterui_dylib*.dll found under target\debug\deps — run the dylib build first'
}

$failed = $false
foreach ($dll in ($dlls | Sort-Object Name)) {
    $count = (& $readobj --coff-exports $dll.FullName | Select-String 'Name:').Count
    $status = 'ok'
    if ($count -gt $budget) {
        $status = 'OVER BUDGET'
        $failed = $true
    }
    Write-Host "$count`t$($dll.Name)`t$status"
}
if ($failed) {
    exit 1
}
