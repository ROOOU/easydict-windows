"""Inspector for a hung Windows GUI process.

No debugger is required. For every thread it reads the current context, scans the
committed part of the stack for return addresses, maps them to module + RVA, and
resolves symbols from the PDB sitting next to each module.

    python hang-inspect.py <pid> [--stack-bytes 262144] [--max-frames 80]

Written for the Easydict Windows port: the UI thread wedges after startup and the
only way to see where is to look at the stack.
"""

from __future__ import annotations

import argparse
import ctypes
import os
import sys
from ctypes import wintypes

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)
dbghelp = ctypes.WinDLL("dbghelp", use_last_error=True)

PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
THREAD_SUSPEND_RESUME = 0x0002
THREAD_GET_CONTEXT = 0x0008
THREAD_QUERY_INFORMATION = 0x0040

CONTEXT_AMD64 = 0x00100000
CONTEXT_CONTROL = 0x00000001
CONTEXT_INTEGER = 0x00000002
CONTEXT_FLAGS = CONTEXT_AMD64 | CONTEXT_CONTROL | CONTEXT_INTEGER

MEM_COMMIT = 0x1000

SYMOPT_UNDNAME = 0x00000002
SYMOPT_DEFERRED_LOADS = 0x00000004
SYMOPT_LOAD_LINES = 0x00000010
SYMOPT_FAIL_CRITICAL_ERRORS = 0x00000200
SYMOPT_AUTO_PUBLICS = 0x00010000

# sizeof(SYMBOL_INFO) as dbghelp defines it: offsetof(Name) + one wchar, rounded
# up to the structure alignment. It must be reported exactly; passing the size of
# an expanded buffer makes SymFromAddr fail with ERROR_INVALID_PARAMETER.
SYMBOL_INFO_SIZE = 88


class CONTEXT64(ctypes.Structure):
    _fields_ = [
        ("P1Home", ctypes.c_uint64), ("P2Home", ctypes.c_uint64),
        ("P3Home", ctypes.c_uint64), ("P4Home", ctypes.c_uint64),
        ("P5Home", ctypes.c_uint64), ("P6Home", ctypes.c_uint64),
        ("ContextFlags", ctypes.c_uint32), ("MxCsr", ctypes.c_uint32),
        ("SegCs", ctypes.c_uint16), ("SegDs", ctypes.c_uint16),
        ("SegEs", ctypes.c_uint16), ("SegFs", ctypes.c_uint16),
        ("SegGs", ctypes.c_uint16), ("SegSs", ctypes.c_uint16),
        ("EFlags", ctypes.c_uint32),
        ("Dr0", ctypes.c_uint64), ("Dr1", ctypes.c_uint64),
        ("Dr2", ctypes.c_uint64), ("Dr3", ctypes.c_uint64),
        ("Dr6", ctypes.c_uint64), ("Dr7", ctypes.c_uint64),
        ("Rax", ctypes.c_uint64), ("Rcx", ctypes.c_uint64),
        ("Rdx", ctypes.c_uint64), ("Rbx", ctypes.c_uint64),
        ("Rsp", ctypes.c_uint64), ("Rbp", ctypes.c_uint64),
        ("Rsi", ctypes.c_uint64), ("Rdi", ctypes.c_uint64),
        ("R8", ctypes.c_uint64), ("R9", ctypes.c_uint64),
        ("R10", ctypes.c_uint64), ("R11", ctypes.c_uint64),
        ("R12", ctypes.c_uint64), ("R13", ctypes.c_uint64),
        ("R14", ctypes.c_uint64), ("R15", ctypes.c_uint64),
        ("Rip", ctypes.c_uint64),
        ("FltSave", ctypes.c_byte * 512),
        ("VectorRegister", ctypes.c_byte * (26 * 16)),
        ("VectorControl", ctypes.c_uint64),
        ("DebugControl", ctypes.c_uint64),
        ("LastBranchToRip", ctypes.c_uint64),
        ("LastBranchFromRip", ctypes.c_uint64),
        ("LastExceptionToRip", ctypes.c_uint64),
        ("LastExceptionFromRip", ctypes.c_uint64),
    ]


class MEMORY_BASIC_INFORMATION64(ctypes.Structure):
    _fields_ = [
        ("BaseAddress", ctypes.c_uint64),
        ("AllocationBase", ctypes.c_uint64),
        ("AllocationProtect", ctypes.c_uint32),
        ("__alignment1", ctypes.c_uint32),
        ("RegionSize", ctypes.c_uint64),
        ("State", ctypes.c_uint32),
        ("Protect", ctypes.c_uint32),
        ("Type", ctypes.c_uint32),
        ("__alignment2", ctypes.c_uint32),
    ]


class SYMBOL_INFOW(ctypes.Structure):
    _fields_ = [
        ("SizeOfStruct", ctypes.c_uint32),
        ("TypeIndex", ctypes.c_uint32),
        ("Reserved", ctypes.c_uint64 * 2),
        ("Index", ctypes.c_uint32),
        ("Size", ctypes.c_uint32),
        ("ModBase", ctypes.c_uint64),
        ("Flags", ctypes.c_uint32),
        ("Value", ctypes.c_uint64),
        ("Address", ctypes.c_uint64),
        ("Register", ctypes.c_uint32),
        ("Scope", ctypes.c_uint32),
        ("Tag", ctypes.c_uint32),
        ("NameLen", ctypes.c_uint32),
        ("MaxNameLen", ctypes.c_uint32),
        ("Name", ctypes.c_wchar * 512),
    ]


class MODULEINFO(ctypes.Structure):
    _fields_ = [
        ("lpBaseOfDll", ctypes.c_void_p),
        ("SizeOfImage", wintypes.DWORD),
        ("EntryPoint", ctypes.c_void_p),
    ]


class IMAGEHLP_LINEW64(ctypes.Structure):
    _fields_ = [
        ("SizeOfStruct", ctypes.c_uint32),
        ("Key", ctypes.c_void_p),
        ("LineNumber", ctypes.c_uint32),
        ("FileName", ctypes.c_wchar_p),
        ("Address", ctypes.c_uint64),
    ]


kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.OpenThread.restype = wintypes.HANDLE
kernel32.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.ReadProcessMemory.argtypes = [
    wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t,
    ctypes.POINTER(ctypes.c_size_t),
]
kernel32.VirtualQueryEx.argtypes = [
    wintypes.HANDLE, ctypes.c_void_p,
    ctypes.POINTER(MEMORY_BASIC_INFORMATION64), ctypes.c_size_t,
]
kernel32.GetThreadContext.argtypes = [wintypes.HANDLE, ctypes.c_void_p]
kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]

psapi.EnumProcessModulesEx.argtypes = [
    wintypes.HANDLE, ctypes.POINTER(wintypes.HMODULE), wintypes.DWORD,
    ctypes.POINTER(wintypes.DWORD), wintypes.DWORD,
]
psapi.GetModuleBaseNameW.argtypes = [
    wintypes.HANDLE, wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD,
]
psapi.GetModuleFileNameExW.argtypes = [
    wintypes.HANDLE, wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD,
]
psapi.GetModuleInformation.argtypes = [
    wintypes.HANDLE, wintypes.HMODULE, ctypes.POINTER(MODULEINFO), wintypes.DWORD,
]

dbghelp.SymInitializeW.argtypes = [wintypes.HANDLE, wintypes.LPCWSTR, wintypes.BOOL]
dbghelp.SymLoadModuleExW.restype = ctypes.c_uint64
dbghelp.SymLoadModuleExW.argtypes = [
    wintypes.HANDLE, wintypes.HANDLE, wintypes.LPCWSTR, wintypes.LPCWSTR,
    ctypes.c_uint64, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD,
]
dbghelp.SymFromAddrW.argtypes = [
    wintypes.HANDLE, ctypes.c_uint64, ctypes.POINTER(ctypes.c_uint64),
    ctypes.POINTER(SYMBOL_INFOW),
]
dbghelp.SymGetLineFromAddrW64.argtypes = [
    wintypes.HANDLE, ctypes.c_uint64, ctypes.POINTER(wintypes.DWORD),
    ctypes.POINTER(IMAGEHLP_LINEW64),
]


class Module:
    def __init__(self, name, path, base, size):
        self.name = name
        self.path = path
        self.base = base
        self.size = size

    @property
    def end(self):
        return self.base + self.size


class Inspector:
    def __init__(self, pid):
        self.pid = pid
        self.hprocess = kernel32.OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not self.hprocess:
            raise OSError(f"OpenProcess({pid}) failed: {ctypes.get_last_error()}")
        self.modules = self._modules()
        self._symbols_ready = False

    # -- process / module plumbing -------------------------------------------
    def _modules(self):
        needed = wintypes.DWORD(0)
        psapi.EnumProcessModulesEx(
            self.hprocess, None, 0, ctypes.byref(needed), 0x03)
        count = needed.value // ctypes.sizeof(wintypes.HMODULE) + 16
        array = (wintypes.HMODULE * count)()
        returned = wintypes.DWORD(0)
        if not psapi.EnumProcessModulesEx(
                self.hprocess, array, ctypes.sizeof(array), ctypes.byref(returned), 0x03):
            raise OSError(f"EnumProcessModulesEx failed: {ctypes.get_last_error()}")
        modules = []
        for handle in array[: returned.value // ctypes.sizeof(wintypes.HMODULE)]:
            base = ctypes.cast(handle, ctypes.c_void_p).value
            name_buf = ctypes.create_unicode_buffer(260)
            psapi.GetModuleBaseNameW(self.hprocess, handle, name_buf, 260)
            path_buf = ctypes.create_unicode_buffer(1024)
            psapi.GetModuleFileNameExW(self.hprocess, handle, path_buf, 1024)
            info = MODULEINFO()
            size = 0
            if psapi.GetModuleInformation(
                    self.hprocess, handle, ctypes.byref(info), ctypes.sizeof(info)):
                size = info.SizeOfImage
            modules.append(Module(name_buf.value, path_buf.value, base, size))
        modules.sort(key=lambda m: m.base)
        return modules

    def module_of(self, address):
        for module in self.modules:
            if module.base <= address < module.end:
                return module
        return None

    def describe(self, address):
        module = self.module_of(address)
        if module is None:
            return f"0x{address:012X}"
        return f"{module.name}+0x{address - module.base:X}"

    def _prepare_symbols(self, sympath=None, all_modules=False):
        if self._symbols_ready:
            return
        dbghelp.SymSetOptions(
            SYMOPT_UNDNAME | SYMOPT_DEFERRED_LOADS | SYMOPT_LOAD_LINES
            | SYMOPT_FAIL_CRITICAL_ERRORS | SYMOPT_AUTO_PUBLICS)
        dbghelp.SymInitializeW(self.hprocess, sympath, False)
        for module in self.modules:
            pdb = os.path.splitext(module.path)[0] + ".pdb"
            if all_modules or os.path.exists(pdb):
                dbghelp.SymLoadModuleExW(
                    self.hprocess, None, module.path, module.name,
                    module.base, module.size, None, 0)
        self._symbols_all = all_modules
        self._symbols_ready = True

    def symbol(self, address):
        self._prepare_symbols()
        displacement = ctypes.c_uint64(0)
        info = SYMBOL_INFOW()
        info.SizeOfStruct = SYMBOL_INFO_SIZE
        info.MaxNameLen = 512
        name = None
        if dbghelp.SymFromAddrW(
                self.hprocess, address, ctypes.byref(displacement), ctypes.byref(info)):
            name = f"{info.Name}+0x{displacement.value:X}"
        line_text = None
        line = IMAGEHLP_LINEW64()
        line.SizeOfStruct = ctypes.sizeof(IMAGEHLP_LINEW64)
        line_displacement = wintypes.DWORD(0)
        if dbghelp.SymGetLineFromAddrW64(
                self.hprocess, address, ctypes.byref(line_displacement), ctypes.byref(line)):
            if line.FileName:
                line_text = f"{os.path.basename(line.FileName)}:{line.LineNumber}"
        return name, line_text

    # -- memory / thread plumbing -------------------------------------------
    def read(self, address, size):
        buffer = ctypes.create_string_buffer(size)
        read = ctypes.c_size_t(0)
        ok = kernel32.ReadProcessMemory(
            self.hprocess, ctypes.c_void_p(address), buffer, size, ctypes.byref(read))
        if not ok:
            return b""
        return buffer.raw[: read.value]

    def stack_region(self, address):
        info = MEMORY_BASIC_INFORMATION64()
        if not kernel32.VirtualQueryEx(
                self.hprocess, ctypes.c_void_p(address), ctypes.byref(info),
                ctypes.sizeof(info)):
            return None
        return info

    def thread_ids(self):
        TH32CS_SNAPTHREAD = 0x00000004
        class THREADENTRY32(ctypes.Structure):
            _fields_ = [
                ("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD),
                ("th32ThreadID", wintypes.DWORD), ("th32OwnerProcessID", wintypes.DWORD),
                ("tpBasePri", wintypes.LONG), ("tpDeltaPri", wintypes.LONG),
                ("dwFlags", wintypes.DWORD),
            ]
        snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
        entry = THREADENTRY32()
        entry.dwSize = ctypes.sizeof(THREADENTRY32)
        ids = []
        if kernel32.Thread32First(snapshot, ctypes.byref(entry)):
            while True:
                if entry.th32OwnerProcessID == self.pid:
                    ids.append(entry.th32ThreadID)
                if not kernel32.Thread32Next(snapshot, ctypes.byref(entry)):
                    break
        kernel32.CloseHandle(snapshot)
        return ids

    def context_of(self, thread_id):
        handle = kernel32.OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
            False, thread_id)
        if not handle:
            return None, None
        raw = ctypes.create_string_buffer(ctypes.sizeof(CONTEXT64) + 16)
        offset = (-ctypes.addressof(raw)) % 16
        pointer = ctypes.addressof(raw) + offset
        context = CONTEXT64.from_address(pointer)
        kernel32.SuspendThread(handle)
        try:
            context.ContextFlags = CONTEXT_FLAGS
            if not kernel32.GetThreadContext(handle, ctypes.c_void_p(pointer)):
                return handle, None
            snapshot = CONTEXT64.from_buffer_copy(
                ctypes.string_at(pointer, ctypes.sizeof(CONTEXT64)))
        finally:
            kernel32.ResumeThread(handle)
        return handle, snapshot

    def stack_frames(self, rsp, max_bytes, max_frames):
        """Return (offset, address) pairs of plausible return addresses."""
        frames = []
        chunk = 4096
        gathered = b""
        offset = 0
        while offset < max_bytes:
            # Never ask for more than the committed region holds: ReadProcessMemory
            # fails outright if any byte of the range is unreadable, and the top of
            # a shallow stack ends mid-page.
            region = self.stack_region(rsp + offset)
            if region is None or region.State != MEM_COMMIT:
                break
            region_end = region.BaseAddress + region.RegionSize
            available = min(chunk, max_bytes - offset, region_end - (rsp + offset))
            if available <= 0:
                break
            block = self.read(rsp + offset, available)
            if not block:
                break
            gathered += block
            offset += len(block)
            if len(block) < available:
                break
        limit = len(gathered) - 8
        position = 0
        while position <= limit and len(frames) < max_frames:
            value = int.from_bytes(gathered[position:position + 8], "little")
            if value > 0x10000 and self.module_of(value) is not None:
                frames.append((position, value))
            position += 8
        return frames, len(gathered)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("pid", type=int)
    parser.add_argument("--stack-bytes", type=int, default=262144)
    parser.add_argument("--max-frames", type=int, default=80)
    parser.add_argument("--all-frames", action="store_true",
                        help="print frames without symbols too")
    parser.add_argument("--thread", type=int, default=None)
    parser.add_argument("--sympath", default=None,
                        help="dbghelp search path, e.g. "
                             "srv*C:\\symbols*https://msdl.microsoft.com/download/symbols")
    parser.add_argument("--no-symbols", action="store_true")
    arguments = parser.parse_args()

    inspector = Inspector(arguments.pid)
    if not arguments.no_symbols:
        inspector._prepare_symbols(arguments.sympath, all_modules=True)
    print(f"process {arguments.pid}: {len(inspector.modules)} modules")
    print(f"python arch: {ctypes.sizeof(ctypes.c_void_p) * 8}-bit")
    print()

    for thread_id in inspector.thread_ids():
        if arguments.thread is not None and thread_id != arguments.thread:
            continue
        handle, context = inspector.context_of(thread_id)
        if handle is None:
            print(f"thread {thread_id}: OpenThread failed")
            continue
        if context is None:
            print(f"thread {thread_id}: GetThreadContext failed")
            kernel32.CloseHandle(handle)
            continue

        rip, rsp = context.Rip, context.Rsp
        rip_symbol, rip_line = inspector.symbol(rip)
        region = inspector.stack_region(rsp)
        region_text = ""
        if region is not None:
            region_text = (
                f" region=0x{region.BaseAddress:X}+0x{region.RegionSize:X}"
                f" state=0x{region.State:X} protect=0x{region.Protect:X}")

        print(f"==== thread {thread_id}")
        print(f"  RIP {inspector.describe(rip)}"
              + (f"  [{rip_symbol}]" if rip_symbol else "")
              + (f"  ({rip_line})" if rip_line else ""))
        print(f"  RSP 0x{rsp:X}{region_text}")

        frames, read_bytes = inspector.stack_frames(
            rsp, arguments.stack_bytes, arguments.max_frames)
        print(f"  stack read: {read_bytes} bytes, {len(frames)} candidates")
        for offset, address in frames:
            module = inspector.module_of(address)
            name, line = (None, None)
            if module is not None:
                name, line = inspector.symbol(address)
            if name is None and not arguments.all_frames:
                if module is not None and module.name.lower() in (
                        "ntdll.dll", "kernelbase.dll", "kernel32.dll"):
                    continue
            text = f"    +0x{offset:05X}  {inspector.describe(address)}"
            if name:
                text += f"  {name}"
            if line:
                text += f"  ({line})"
            print(text)
        print()
        kernel32.CloseHandle(handle)


if __name__ == "__main__":
    sys.exit(main())
