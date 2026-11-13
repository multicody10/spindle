!macro NSIS_HOOK_POSTINSTALL
  ; Refresh the existing desktop shortcut after files are installed. This macro
  ; also runs during updater installs, where Tauri skips shortcut recreation.
  ; Do not create a desktop shortcut if the user previously chose not to have one.
  SetShellVarContext all
  IfFileExists "$DESKTOP\${PRODUCTNAME}.lnk" 0 spindle_desktop_shortcut_done
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
spindle_desktop_shortcut_done:
  SetShellVarContext current
!macroend
