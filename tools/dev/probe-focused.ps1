param(
    [string[]]$ProbeArgs = @(),
    [switch]$Keep,
    [switch]$RichText
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class F {
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

$form = New-Object System.Windows.Forms.Form
$form.Text = 'Selection probe target'
$form.Width = 560; $form.Height = 200; $form.TopMost = $true
if ($RichText) {
    # RichEdit publishes a UI Automation TextPattern, unlike the plain EDIT
    # control, so it exercises the UIA path instead of the clipboard fallback.
    $box = New-Object System.Windows.Forms.RichTextBox
} else {
    $box = New-Object System.Windows.Forms.TextBox
}
$box.Multiline = $true; $box.Dock = 'Fill'; $box.Text = 'hello selection harness'
$form.Controls.Add($box)
$form.Show()
[F]::Activate($form.Handle)
$box.Focus(); $box.SelectAll()
[System.Windows.Forms.Application]::DoEvents()
Start-Sleep -Milliseconds 500
Write-Output "foreground is target: $([F]::GetForegroundWindow() -eq $form.Handle)"
Write-Output "selection inside target: '$($box.SelectedText)'"

$probe = (Resolve-Path "$PSScriptRoot\..\..\src-tauri\target\debug\examples\selection-probe.exe").Path
$out = "$env:TEMP\dsh-sel-test\focused-probe.txt"
$err = "$env:TEMP\dsh-sel-test\focused-probe.err"
Remove-Item $out, $err -ErrorAction SilentlyContinue
$startArgs = @{ FilePath = $probe; WindowStyle = 'Hidden'; RedirectStandardOutput = $out; RedirectStandardError = $err; PassThru = $true }
if ($ProbeArgs.Count -gt 0) { $startArgs.ArgumentList = @($ProbeArgs) }
$p = Start-Process @startArgs
# The form has to keep pumping messages while the probe runs: Ctrl+C is
# delivered to this thread's queue, and a blocking WaitForExit would only let
# the copy happen after the probe had already timed out.
while (-not $p.HasExited) {
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Milliseconds 15
}
[System.Windows.Forms.Application]::DoEvents()
Start-Sleep -Milliseconds 200
Write-Output "foreground still target: $([F]::GetForegroundWindow() -eq $form.Handle)"
Write-Output '--- probe stdout ---'
Get-Content $out -ErrorAction SilentlyContinue | ForEach-Object { $_ }
Write-Output '--- probe stderr ---'
Get-Content $err -ErrorAction SilentlyContinue | ForEach-Object { $_ }
Write-Output "clipboard after probe: '$([System.Windows.Forms.Clipboard]::GetText())'"

if (-not $Keep) { $form.Close() }
