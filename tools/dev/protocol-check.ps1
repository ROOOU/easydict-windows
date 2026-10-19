# Checks the easydict:// protocol registration and the cold-start link path on a
# real desktop, against the built release binary:
#
#   * a plain start registers `HKCU\Software\Classes\easydict` for this user,
#   * `ShellExecute` on a link then starts the app — the same path Explorer and a
#     browser take, including Windows' own URL normalisation,
#   * the process routed the link, which its log records,
#   * the query window really appeared and the text reached a query,
#   * and `app.exe --unregister-url-scheme` removes the registration again.
#
# Run it from a Windows PowerShell 5.1 session (no PowerShell 7 is installed):
#
#   & ./tools/dev/protocol-check.ps1 -OutputPath ../docs/protocol-check-<date>.json
#
# The length of the run is dominated by one WebView2 start-up. The app is left
# registered on purpose: the next start of Easydict registers it again, which
# is also what makes the check repeatable.

[CmdletBinding()]
param(
  [string]$Exe = (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "src-tauri\target\release\app.exe"),
  [string]$OutputPath = "",
  [int]$TimeoutSeconds = 45,
  [int]$SettleSeconds = 6
)

$ErrorActionPreference = "Stop"
$schemeKey = "HKCU:\Software\Classes\easydict"
$commandKey = Join-Path $schemeKey "shell\open\command"
$logPath = Join-Path $env:LOCALAPPDATA "com.easydict.windows\logs\Easydict.log"
$link = "easydict://query?text=cold%20start%20probe"

if ($OutputPath -and (Test-Path -LiteralPath $OutputPath)) {
  throw "refusing to overwrite $OutputPath"
}
if (-not (Test-Path -LiteralPath $Exe)) {
  throw "no release binary at $Exe; build it with: cargo build --release"
}
$Exe = (Resolve-Path -LiteralPath $Exe).Path

# A second instance is forwarded to the first one, which would make this a
# warm-start measurement instead of a cold start.
$running = @(Get-Process -Name "app" -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
  throw "an Easydict process is already running (pid $($running.Id -join ', ')); close it first"
}

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class WindowProbe {
  delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc callback, IntPtr extra);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int count);
  public static List<string> VisibleTitles() {
    var titles = new List<string>();
    EnumWindows((hWnd, extra) => {
      if (!IsWindowVisible(hWnd)) return true;
      var text = new StringBuilder(512);
      GetWindowTextW(hWnd, text, text.Capacity);
      if (text.Length > 0) titles.Add(text.ToString());
      return true;
    }, IntPtr.Zero);
    return titles;
  }
}
"@

function Read-RegisteredCommand {
  if (-not (Test-Path -LiteralPath $commandKey)) { return $null }
  return (Get-ItemProperty -LiteralPath $commandKey).'(default)'
}
function Invoke-Exe([string[]]$Arguments) {
  # Windows PowerShell 5.1 rejects an empty `-ArgumentList` collection, so a
  # plain start has to be issued without the parameter at all.
  if ($Arguments -and $Arguments.Count -gt 0) {
    return Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru
  }
  return Start-Process -FilePath $Exe -PassThru
}

# Opens the link the way Explorer, a browser or another program does: through
# `ShellExecute`, so the registered handler is looked up and the URL goes
# through Windows' own normalisation. Passing the link as an argument instead —
# which this script used to do — skips that step and tests a spelling no real
# caller produces: `ShellExecute` turns `easydict://query?text=…` into
# `easydict://query/?text=…` before the handler sees it, and a handler that only
# matched the tidy form looked correct here while every real link was dropped.
function Invoke-Link([string]$Url) {
  Start-Process -FilePath $Url
}

$startedAt = Get-Date
$result = [ordered]@{
  status = "running"
  startedAt = $startedAt.ToString("o")
  exe = $Exe
  exeSha256 = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
  link = $link
  launchedThroughShellExecute = $true
  registeredBefore = Read-RegisteredCommand
  coldStart = $true
  windowAppeared = $false
  logLineFound = $false
  queryExecuted = $false
  registeredCommand = $null
  registrationRemoved = $false
  secondRemovalExitCode = $null
  registeredAfterPlainStart = $false
  processStopped = $false
  logTail = @()
  elapsedSeconds = 0
  notes = @()
}

$logStart = if (Test-Path -LiteralPath $logPath) { (Get-Item -LiteralPath $logPath).Length } else { 0 }

# `Get-Content` decorates its strings with provider metadata (`PSPath`, and a
# `PSProvider` graph that drags in every loaded module), which turns a report
# into dozens of megabytes. Keep the text only.
function Read-LogTail([int]$Count) {
  if (-not (Test-Path -LiteralPath $logPath)) { return @() }
  return @(Get-Content -LiteralPath $logPath -Tail $Count | ForEach-Object { [string]$_ })
}

# The scheme has to be registered before a link can be opened through it, and
# the registration is written by a start of the app. A plain start is therefore
# what arms the check — and its process is stopped again so the link below is a
# genuine cold start rather than a second instance handed to a running app.
$arm = Invoke-Exe @()
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $schemeKey)) {
  Start-Sleep -Milliseconds 300
}
if (-not (Test-Path -LiteralPath $schemeKey)) {
  throw "the app did not register $schemeKey, so no link can be opened through it"
}
if (-not $arm.HasExited) {
  Stop-Process -Id $arm.Id -Force
  $arm.WaitForExit(15000) | Out-Null
}
$result.registeredBefore = Read-RegisteredCommand

# Wait for the process the shell started, which is not a child of this script.
function Wait-ForAppProcess([int]$Seconds) {
  $end = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $end) {
    $found = @(Get-Process -Name "app" -ErrorAction SilentlyContinue)
    if ($found.Count -gt 0) { return $found[0] }
    Start-Sleep -Milliseconds 200
  }
  return $null
}

Invoke-Link $link
$process = Wait-ForAppProcess $TimeoutSeconds
if (-not $process) {
  throw "the shell did not start the app for $link"
}
try {
  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    if (-not $result.registeredCommand) { $result.registeredCommand = Read-RegisteredCommand }
    if (-not $result.windowAppeared) {
      $titles = [WindowProbe]::VisibleTitles()
      $result.windowAppeared = @($titles | Where-Object { $_ -like "Easydict*" }).Count -gt 0
    }
    if (-not $result.logLineFound -or -not $result.queryExecuted) {
      $tail = Read-LogTail 40
      $result.logLineFound = @($tail | Where-Object { $_ -match "deep link: query" }).Count -gt 0
      # The link is only really handled once the queued text reached a query:
      # a window alone proves nothing about the payload.
      $result.queryExecuted = @($tail | Where-Object { $_ -match "query: window=fixed" }).Count -gt 0
    }
    if ($result.registeredCommand -and $result.windowAppeared -and $result.logLineFound -and $result.queryExecuted) { break }
    Start-Sleep -Milliseconds 500
    if ($process.HasExited) {
      $result.notes += "the app exited on its own with code $($process.ExitCode)"
      break
    }
  }

  $result.logTail = Read-LogTail 12
  Start-Sleep -Seconds $SettleSeconds

  $expected = "`"$Exe`" `"%1`""
  if ($result.registeredCommand -ne $expected) {
    $result.notes += "registered command is '$($result.registeredCommand)', expected '$expected'"
  }
} finally {
  if (-not $process.HasExited) {
    Stop-Process -Id $process.Id -Force
    $process.WaitForExit(15000) | Out-Null
  }
  $result.processStopped = $process.HasExited
}

# Removal has to work without a registry editor, and it must be repeatable.
$remove = Invoke-Exe @("--unregister-url-scheme")
$remove.WaitForExit(20000) | Out-Null
$result.registrationRemoved = -not (Test-Path -LiteralPath $schemeKey)
if (-not $result.registrationRemoved) {
  $result.notes += "the key survived --unregister-url-scheme (exit code $($remove.ExitCode))"
}
$again = Invoke-Exe @("--unregister-url-scheme")
$again.WaitForExit(20000) | Out-Null
$result.secondRemovalExitCode = $again.ExitCode

# A plain start — no link on the command line — has to register the protocol
# again. That is what repairs a registration after the app moves, and it is how
# this check leaves the scheme usable instead of removed.
$plain = Invoke-Exe @()
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $schemeKey)) {
  Start-Sleep -Milliseconds 300
}
$result.registeredAfterPlainStart = Test-Path -LiteralPath $schemeKey
if (-not $plain.HasExited) {
  Stop-Process -Id $plain.Id -Force
  $plain.WaitForExit(15000) | Out-Null
}
if (-not $result.registeredAfterPlainStart) {
  $result.notes += "a plain start did not restore the registration"
}

$result.elapsedSeconds = [math]::Round(((Get-Date) - $startedAt).TotalSeconds, 1)
$result.status = if ($result.windowAppeared -and $result.logLineFound -and $result.queryExecuted -and
  $result.registrationRemoved -and $result.registeredAfterPlainStart -and $result.processStopped) {
  "completed"
} else {
  "incomplete"
}

$json = $result | ConvertTo-Json -Depth 6
if ($OutputPath) {
  $json | Set-Content -LiteralPath $OutputPath -Encoding UTF8
  Write-Output "wrote $OutputPath"
}
$json
