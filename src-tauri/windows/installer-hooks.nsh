!macro NSIS_HOOK_POSTINSTALL
  ; Tauri's /UPDATE installer mode intentionally skips shortcut recreation.
  ; Recreate the desktop link after the new executable is in place so its
  ; icon location always points to the current executable resource.
  SetShellVarContext all
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
  SetShellVarContext current
!macroend
