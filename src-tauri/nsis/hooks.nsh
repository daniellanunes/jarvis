; Installer hooks.
;
; The installer runs elevated once (perMachine). That is when Jarvis registers
; the scheduled task that lets it start with admin rights afterwards without a
; UAC prompt, and turns "start with Windows" on for a first install
; (elevation.rs). The uninstaller removes both before the files go.
;
; The app stages openjarvis-hook.exe into %LOCALAPPDATA%\OpenJarvis\bin at launch, so the
; installer never recorded it and the default uninstaller leaves it behind. The
; inbox and the log live in the same place and are ours too.
;
; Claude Code's own settings.json is deliberately NOT touched here: it belongs to
; the user, it may contain hooks from other tools, and rewriting somebody's
; config from an uninstaller with no diff and no consent is exactly what the rest
; of this app goes out of its way not to do. A relay that is gone exits 0 without
; printing anything, so a leftover entry costs nothing beyond a dead path.

!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --register-elevation'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --unregister-elevation'
  RMDir /r "$LOCALAPPDATA\OpenJarvis\bin"
  RMDir /r "$LOCALAPPDATA\OpenJarvis\inbox"
  Delete "$LOCALAPPDATA\OpenJarvis\openjarvis.log"
!macroend
