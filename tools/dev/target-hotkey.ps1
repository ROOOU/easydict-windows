param(
    [string]$Out = "$env:TEMP\dsh-sel-test\target-hotkey.png",
    [string]$Hotkey = 'Alt+D',
    [int]$WaitMs = 6000,
    [string]$ClipboardMarker = 'EASYDICT CLIPBOARD MARKER'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class T {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] i, int size);
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION u; }
    [StructLayout(LayoutKind.Explicit, Size=32)] public struct INPUTUNION { [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint flags; public uint time; public IntPtr extra; }
    public static void Key(ushort vk, bool up) {
        var i = new INPUT[1]; i[0].type = 1; i[0].u.ki.wVk = vk; i[0].u.ki.flags = up ? 2u : 0u;
        SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
    }
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

function Send-Combo([string]$combo) {
    $vks = @()
    foreach ($p in ($combo -split '\+')) {
        $vks += switch ($p.Trim().ToUpper()) {
            'ALT' { 0x12 } 'CTRL' { 0x11 } 'SHIFT' { 0x10 } 'WIN' { 0x5B }
            default { [int][char]$p.Trim().ToUpper() }
        }
    }
    foreach ($vk in $vks) { [T]::Key([uint16]$vk, $false); Start-Sleep -Milliseconds 40 }
    for ($i = $vks.Count - 1; $i -ge 0; $i--) { [T]::Key([uint16]$vks[$i], $true); Start-Sleep -Milliseconds 40 }
}

$form = New-Object System.Windows.Forms.Form
$form.Text = 'Selection target'
$form.Width = 640; $form.Height = 200; $form.TopMost = $true
$form.Left = 120; $form.Top = 120
$box = New-Object System.Windows.Forms.TextBox
$box.Multiline = $true; $box.Dock = 'Fill'; $box.Text = 'hello selection harness'
$form.Controls.Add($box)
$form.Show()
[T]::Activate($form.Handle)
$box.Focus(); $box.SelectAll()
[System.Windows.Forms.Application]::DoEvents()
Start-Sleep -Milliseconds 400
for ($i = 0; $i -lt 10; $i++) {
    try { [System.Windows.Forms.Clipboard]::SetText($ClipboardMarker); break }
    catch { Start-Sleep -Milliseconds 200 }
}
Write-Output "foreground is target: $([T]::GetForegroundWindow() -eq $form.Handle)"

Send-Combo $Hotkey
$deadline = (Get-Date).AddMilliseconds($WaitMs)
while ((Get-Date) -lt $deadline) {
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Milliseconds 100
}
[System.Windows.Forms.Application]::DoEvents()

$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($bmp)
$graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bmp.Size)
$graphics.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output "screenshot: $Out"
Write-Output "clipboard after hotkey: '$([System.Windows.Forms.Clipboard]::GetText())'"
$form.Close()
