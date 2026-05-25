!macro NSIS_HOOK_PREINSTALL
  ClearErrors
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM "codex-quota-widget.exe" /T'
  ClearErrors
  Sleep 800
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ClearErrors
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM "codex-quota-widget.exe" /T'
  ClearErrors
  Sleep 800
!macroend
