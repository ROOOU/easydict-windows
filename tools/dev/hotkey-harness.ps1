param(
    [string]$App = "$PSScriptRoot\..\..\src-tauri\target\release\app.exe",
    [string]$OutDir = "$env:TEMP\dsh-sel-test",
    [string]$Hotkey = 'Alt+D',
    [int]$WaitMs = 5000,
    [switch]$KeepRunning
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class Api {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();

    public static void Activate(IntPtr hwnd) {
        var fg = GetForegroundWindow();
        uint pid;
        uint fgThread = fg == IntPtr.Zero ? 0 : GetWindowThreadProcessId(fg, out pid);
        uint me = GetCurrentThreadId();
        if (fgThread != 0 && fgThread != me) AttachThreadInput(me, fgThread, true);
        ShowWindow(hwnd, 9);
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if (fgThread != 0 && fgThread != me) AttachThreadInput(me, fgThread, false);
    }

    public static List<string> Windows(uint pid) {
        var list = new List<string>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid) {
                var t = new StringBuilder(256); GetWindowTextW(h, t, 256);
                var c = new StringBuilder(256); GetClassNameW(h, c, 256);
                RECT r; GetWindowRect(h, out r);
                list.Add(string.Format("hwnd={0} visible={1} class={2} title={3} rect={4},{5},{6},{7}",
                    h, IsWindowVisible(h), c, t, r.L, r.T, r.R, r.B));
            }
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
"@

function Send-Combo([string]$combo) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Keys {
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] i, int size);
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION u; }
    [StructLayout(LayoutKind.Explicit, Size=32)] public struct INPUTUNION { [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint flags; public uint time; public IntPtr extra; }
    public static void Key(ushort vk, bool up) {
        var i = new INPUT[1];
        i[0].type = 1;
        i[0].u.ki.wVk = vk;
        i[0].u.ki.flags = up ? 2u : 0u;
        SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
    }
}
"@
    $parts = $combo -split '\+'
    $vks = @()
    foreach ($p in $parts) {
        $vks += switch ($p.Trim().ToUpper()) {
            'ALT' { 0x12 } 'CTRL' { 0x11 } 'SHIFT' { 0x10 } 'WIN' { 0x5B }
            default { [int][char]$p.Trim().ToUpper() }
        }
    }
    foreach ($vk in $vks) { [Keys]::Key([uint16]$vk, $false); Start-Sleep -Milliseconds 40 }
    foreach ($vk in ($vks[($vks.Count - 1)..0])) { [Keys]::Key([uint16]$vk, $true); Start-Sleep -Milliseconds 40 }
}

# A plain WinForms text box as the "other application" with a real selection.
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Easydict selection harness'
$form.Width = 640
$form.Height = 200
$form.StartPosition = 'Manual'
$form.Left = 200
$form.Top = 120
$form.TopMost = $true
$box = New-Object System.Windows.Forms.TextBox
$box.Multiline = $true
$box.Dock = 'Fill'
$box.Text = 'hello selection harness'
$form.Controls.Add($box)
$form.Show()
$form.Activate()
$box.Focus()
$box.SelectAll()
[System.Windows.Forms.Application]::DoEvents()
Start-Sleep -Milliseconds 600
[System.Windows.Forms.Application]::DoEvents()
Write-Output "harness foreground: $([Api]::GetForegroundWindow() -eq $form.Handle)"

if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Force $OutDir | Out-Null }
$stamp = Get-Date -Format 'HHmmss'
$stdout = Join-Path $OutDir "app-stdout-$stamp.txt"
$stderr = Join-Path $OutDir "app-stderr-$stamp.txt"
if (Test-Path $stdout) { Remove-Item $stdout -Force }
if (Test-Path $stderr) { Remove-Item $stderr -Force }

Get-Process app -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
$proc = Start-Process -FilePath (Resolve-Path $App).Path -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
Write-Output "app pid: $($proc.Id)"
Start-Sleep -Milliseconds 2500

# Re-activate in case the app took focus while starting.
[Api]::Activate($form.Handle)
$box.Focus()
$box.SelectAll()
[System.Windows.Forms.Application]::DoEvents()
Start-Sleep -Milliseconds 400
Write-Output "foreground before hotkey: $([Api]::GetForegroundWindow() -eq $form.Handle)"

for ($i = 0; $i -lt 10; $i++) {
    try { [System.Windows.Forms.Clipboard]::SetText('EASYDICT CLIPBOARD MARKER'); break }
    catch { Start-Sleep -Milliseconds 200 }
}

Send-Combo $Hotkey
$deadline = (Get-Date).AddMilliseconds($WaitMs)
while ((Get-Date) -lt $deadline) {
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Milliseconds 100
}
[System.Windows.Forms.Application]::DoEvents()

Write-Output '--- app windows ---'
[Api]::Windows([uint32]$proc.Id) | ForEach-Object { Write-Output $_ }
Write-Output "clipboard after hotkey: '$([System.Windows.Forms.Clipboard]::GetText())'"

$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($bmp)
$graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bmp.Size)
$graphics.Dispose()
$shot = Join-Path $OutDir 'after-hotkey.png'
$bmp.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output "screenshot: $shot"

Write-Output '--- app stdout ---'
Get-Content $stdout -ErrorAction SilentlyContinue | Select-Object -Last 60 | ForEach-Object { $_ }
Write-Output '--- app stderr ---'
Get-Content $stderr -ErrorAction SilentlyContinue | Select-Object -Last 20 | ForEach-Object { $_ }

$form.Close()
if (-not $KeepRunning) {
    Start-Sleep -Milliseconds 300
    Get-Process app -ErrorAction SilentlyContinue | Stop-Process -Force
}


