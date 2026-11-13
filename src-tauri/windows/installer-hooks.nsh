!macro NSIS_HOOK_POSTINSTALL
  ; Rebuild the existing desktop shortcut after files are installed.
  ; This also repairs stale icon references during updater installs.
  ; Do not create a desktop shortcut if the user previously chose not to have one.
  SetShellVarContext all
  IfFileExists "$DESKTOP\${PRODUCTNAME}.lnk" 0 spindle_desktop_shortcut_done
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
  ; Notify Explorer that shell icon associations changed.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
spindle_desktop_shortcut_done:
  SetShellVarContext current
!macroend
