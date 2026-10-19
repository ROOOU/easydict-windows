; Easydict for Windows — NSIS installer hooks.
;
; The application registers itself for `easydict://…` under the current user
; every time it starts (see `src/platform/protocol.rs`), which is the Windows
; counterpart of the `CFBundleURLTypes` entry in the macOS `Info.plist`. NSIS
; removes its own registry entries and the installed files, but not values the
; application wrote while it was running, so the protocol key is removed here.
;
; Only a registration owned by this installation is deleted: a portable copy or
; a newer build that registered itself last keeps its key, and deleting one that
; points elsewhere would silently break that copy's links.
;
; An upgrade runs this uninstaller before installing the new version. That is
; harmless — the installed build registers itself again the first time it runs.

!macro NSIS_HOOK_PREUNINSTALL
  ReadRegStr $0 HKCU "Software\Classes\easydict\shell\open\command" ""
  StrCmp $0 '"$INSTDIR\app.exe" "%1"' 0 easydict_registration_kept
    DeleteRegKey HKCU "Software\Classes\easydict"
  easydict_registration_kept:
!macroend
