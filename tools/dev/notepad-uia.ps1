param([switch]$Keep)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class N {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    public static void Activate(IntPtr hwnd) {
        var fg = GetForegroundWindow();
        uint pid;
        uint fgThread = fg == IntPtr.Zero ? 0 : GetWindowThreadProcessId(fg, out pid);
        uint me = GetCurrentThreadId();
        if (fgThread != 0 && fgThread != me) AttachThreadInput(me, fgThread, true);
        ShowWindow(hwnd, 9); BringWindowToTop(hwnd); SetForegroundWindow(hwnd);
        if (fgThread != 0 && fgThread != me) AttachThreadInput(me, fgThread, false);
    }
}
"@

for ($i = 0; $i -lt 10; $i++) {
    try { [System.Windows.Forms.Clipboard]::SetText('CLIPBOARD MARKER BEFORE'); break }
    catch { Start-Sleep -Milliseconds 300 }
}

Get-Process Notepad -ErrorAction SilentlyContinue | ForEach-Object { $_.CloseMainWindow() | Out-Null }
Start-Sleep -Milliseconds 400
Start-Process notepad.exe | Out-Null
$np = $null
for ($i = 0; $i -lt 60 -and $null -eq $np; $i++) {
    Start-Sleep -Milliseconds 150
    $np = Get-Process Notepad -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
}
$hwnd = [IntPtr]$np.MainWindowHandle
[N]::Activate($hwnd)
Start-Sleep -Milliseconds 800
Write-Output "notepad foreground: $([N]::GetForegroundWindow() -eq $hwnd)"
[System.Windows.Forms.SendKeys]::SendWait('hello notepad selection')
Start-Sleep -Milliseconds 400
[System.Windows.Forms.SendKeys]::SendWait('^a')
Start-Sleep -Milliseconds 800
Write-Output "notepad foreground after typing: $([N]::GetForegroundWindow() -eq $hwnd)"

$probe = (Resolve-Path "$PSScriptRoot\..\..\src-tauri\target\debug\examples\selection-probe.exe").Path
$out = "$env:TEMP\dsh-sel-test\notepad-uia.txt"
$err = "$env:TEMP\dsh-sel-test\notepad-uia.err"
Remove-Item $out, $err -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $probe -ArgumentList '--uia' -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err -PassThru
$p.WaitForExit(20000) | Out-Null
Write-Output '--- probe stdout ---'
Get-Content $out -ErrorAction SilentlyContinue | ForEach-Object { $_ }
Write-Output '--- probe stderr ---'
Get-Content $err -ErrorAction SilentlyContinue | ForEach-Object { $_ }
Write-Output "clipboard after probe: '$([System.Windows.Forms.Clipboard]::GetText())'"

if (-not $Keep) {
    $np.CloseMainWindow() | Out-Null
    Start-Sleep -Milliseconds 400
    if (-not $np.HasExited) { $np.Kill() }
}
