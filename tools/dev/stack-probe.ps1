# Reads the current instruction pointer and a raw stack scan for every thread of a
# hung process, mapping each candidate address to a module + RVA. No debugger needed:
# x64 stack scanning finds return addresses, which is enough to name the blocking
# module (and, with a PDB, the blocking function).
#
#   & .\stack-probe.ps1 -ProcessId 1234
param(
  [Parameter(Mandatory = $true)][int]$ProcessId,
  [int]$StackBytes = 65536,
  [int]$MaxThreads = 40,
  [int]$MaxFrames = 60
)

$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class Native {
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern IntPtr OpenThread(uint access, bool inherit, uint threadId);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern int SuspendThread(IntPtr hThread);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern int ResumeThread(IntPtr hThread);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern bool GetThreadContext(IntPtr hThread, IntPtr context);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern bool ReadProcessMemory(IntPtr hProcess, IntPtr baseAddr, byte[] buffer, IntPtr size, out IntPtr read);
  [DllImport("kernel32.dll")] public static extern uint GetLastError();
}
"@

$PROCESS_QUERY_INFORMATION = 0x0400
$PROCESS_VM_READ = 0x0010
$PROCESS_SUSPEND_RESUME = 0x0800
$THREAD_SUSPEND_RESUME = 0x0002
$THREAD_GET_CONTEXT = 0x0008
$THREAD_QUERY_INFORMATION = 0x0040
$CONTEXT_AMD64 = 0x00100000
$CONTEXT_CONTROL = 0x00000001
$CONTEXT_INTEGER = 0x00000002
$CONTEXT_FLAGS = $CONTEXT_AMD64 -bor $CONTEXT_CONTROL -bor $CONTEXT_INTEGER

$process = Get-Process -Id $ProcessId
$hProcess = [Native]::OpenProcess($PROCESS_QUERY_INFORMATION -bor $PROCESS_VM_READ -bor $PROCESS_SUSPEND_RESUME, $false, $ProcessId)
if ($hProcess -eq [IntPtr]::Zero) { throw "OpenProcess failed: $([Native]::GetLastError())" }

# Address ranges of every module, longest first so the best match wins.
$modules = @()
foreach ($m in $process.Modules) {
  $modules += [pscustomobject]@{
    Name  = $m.ModuleName
    Path  = $m.FileName
    Base  = [int64]$m.BaseAddress
    End   = [int64]$m.BaseAddress + [int64]$m.ModuleMemorySize
  }
}

function Resolve-Address([int64]$address) {
  foreach ($m in $modules) {
    if ($address -ge $m.Base -and $address -lt $m.End) {
      return [pscustomobject]@{ Module = $m.Name; Rva = ('0x{0:X}' -f ($address - $m.Base)); Path = $m.Path }
    }
  }
  return $null
}

"process: $($process.ProcessName) ($ProcessId)  responding=$($process.Responding)  threads=$($process.Threads.Count)"
""

$contextSize = 0x1000
$contextBuffer = [Runtime.InteropServices.Marshal]::AllocHGlobal($contextSize)

try {
  $shown = 0
  foreach ($thread in ($process.Threads | Sort-Object Id | Select-Object -First $MaxThreads)) {
    $hThread = [Native]::OpenThread($THREAD_SUSPEND_RESUME -bor $THREAD_GET_CONTEXT -bor $THREAD_QUERY_INFORMATION, $false, [uint32]$thread.Id)
    if ($hThread -eq [IntPtr]::Zero) {
      "thread $($thread.Id): OpenThread failed ($([Native]::GetLastError()))"
      continue
    }

    [void][Native]::SuspendThread($hThread)
    try {
      for ($i = 0; $i -lt $contextSize; $i++) { [Runtime.InteropServices.Marshal]::WriteByte($contextBuffer, $i, 0) }
      [Runtime.InteropServices.Marshal]::WriteInt32($contextBuffer, 0x30, $CONTEXT_FLAGS)
      if (-not [Native]::GetThreadContext($hThread, $contextBuffer)) {
        "thread $($thread.Id): GetThreadContext failed ($([Native]::GetLastError()))"
        continue
      }
      $rip = [Runtime.InteropServices.Marshal]::ReadInt64($contextBuffer, 0xF8)
      $rsp = [Runtime.InteropServices.Marshal]::ReadInt64($contextBuffer, 0x98)

      $at = Resolve-Address $rip
      $where = if ($at) { "$($at.Module)+$($at.Rva)" } else { "0x{0:X}" -f $rip }
      ""
      "==== thread $($thread.Id)  state=$($thread.ThreadState)/$($thread.WaitReason)  cpu=$([math]::Round($thread.TotalProcessorTime.TotalSeconds,3))s"
      "  RIP $where"

      $buffer = New-Object byte[] $StackBytes
      $read = [IntPtr]::Zero
      if ([Native]::ReadProcessMemory($hProcess, [IntPtr]$rsp, $buffer, [IntPtr]$StackBytes, [ref]$read) -and $read.ToInt64() -gt 0) {
        $limit = [int]$read.ToInt64() - 8
        $frames = 0
        for ($offset = 0; $offset -le $limit -and $frames -lt $MaxFrames; $offset += 8) {
          $value = [BitConverter]::ToInt64($buffer, $offset)
          if ($value -le 0x10000) { continue }
          $resolved = Resolve-Address $value
          if ($resolved) {
            "    +0x{0:X4}  {1}+{2}" -f $offset, $resolved.Module, $resolved.Rva
            $frames++
          }
        }
      }
      else {
        "  stack unreadable at 0x{0:X}" -f $rsp
      }
    }
    finally {
      [void][Native]::ResumeThread($hThread)
      [void][Native]::CloseHandle($hThread)
    }
    $shown++
  }
}
finally {
  [Runtime.InteropServices.Marshal]::FreeHGlobal($contextBuffer)
  [void][Native]::CloseHandle($hProcess)
}
