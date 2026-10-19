"""Checks whether dbghelp can resolve app.exe addresses for a running process.

    python symbol-probe.py <pid> 0xB4BB17 0x2581EC ...

Addresses may be given as module RVAs relative to the module whose name is passed
with --module.
"""

from __future__ import annotations

import argparse
import ctypes
import importlib.util
import os
import sys

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location(
    "hanginspect", os.path.join(here, "hang-inspect.py"))
hang = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hang)

parser = argparse.ArgumentParser()
parser.add_argument("pid", type=int)
parser.add_argument("addresses", nargs="+")
parser.add_argument("--module", default="app.exe")
arguments = parser.parse_args()

inspector = hang.Inspector(arguments.pid)
module = next((m for m in inspector.modules if m.name.lower() == arguments.module.lower()), None)
if module is None:
    print(f"module {arguments.module} not found; modules:")
    for m in inspector.modules:
        print(f"  {m.name}  0x{m.base:X}  {m.path}")
    raise SystemExit(1)

print(f"module {module.name}")
print(f"  path {module.path}")
print(f"  base 0x{module.base:X}  size 0x{module.size:X}  end 0x{module.end:X}")
pdb = os.path.splitext(module.path)[0] + ".pdb"
print(f"  pdb  {pdb}  exists={os.path.exists(pdb)}")

dbghelp = hang.dbghelp
dbghelp.SymSetOptions(
    hang.SYMOPT_UNDNAME | hang.SYMOPT_DEFERRED_LOADS | hang.SYMOPT_LOAD_LINES
    | hang.SYMOPT_FAIL_CRITICAL_ERRORS | hang.SYMOPT_AUTO_PUBLICS)
ok = dbghelp.SymInitializeW(inspector.hprocess, None, False)
print(f"SymInitializeW -> {ok} err={ctypes.get_last_error()}")
loaded = dbghelp.SymLoadModuleExW(
    inspector.hprocess, None, module.path, module.name,
    module.base, module.size, None, 0)
print(f"SymLoadModuleExW -> 0x{loaded:X} err={ctypes.get_last_error()}")

for text in arguments.addresses:
    rva = int(text, 16)
    address = module.base + rva
    displacement = ctypes.c_uint64(0)
    info = hang.SYMBOL_INFOW()
    info.SizeOfStruct = hang.SYMBOL_INFO_SIZE
    info.MaxNameLen = 512
    resolved = dbghelp.SymFromAddrW(
        inspector.hprocess, address, ctypes.byref(displacement), ctypes.byref(info))
    print(f"  {text} -> ok={resolved} err={ctypes.get_last_error()} "
          f"name={info.Name!r} disp=0x{displacement.value:X} sizeof={info.SizeOfStruct}")
