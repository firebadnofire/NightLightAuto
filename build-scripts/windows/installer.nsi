; SPDX-License-Identifier: MPL-2.0
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!ifndef APP_VERSION
  !error "APP_VERSION must be supplied by release.py"
!endif
Name "Fullscreen Night Light ${APP_VERSION}"
OutFile "${INSTALLER_OUT}"
InstallDir "$LOCALAPPDATA\Programs\FullscreenNightLight"
InstallDirRegKey HKCU "Software\FullscreenNightLight\Installer" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ManifestDPIAware true
VIProductVersion "${WINDOWS_VERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "Fullscreen Night Light"
VIAddVersionKey /LANG=1033 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "FileDescription" "Fullscreen Night Light Setup"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Licensed under MPL-2.0"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PAYLOAD_DIR}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\FullscreenNightLight"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Fullscreen Night Light requires Windows 11 x64."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "Fullscreen Night Light"
  ; Graceful exit releases owned overrides; never force-terminate the app.
  IfFileExists "$INSTDIR\FullscreenNightLight.exe" 0 stopped
    ExecWait '"$INSTDIR\FullscreenNightLight.exe" --exit' $0
    Sleep 1000
  stopped:
  SetOutPath "$INSTDIR"
  File "${PAYLOAD_DIR}\FullscreenNightLight.exe"
  File "${PAYLOAD_DIR}\README.md"
  File "${PAYLOAD_DIR}\LICENSE"
  File "${PAYLOAD_DIR}\config.example.toml"
  File "${PAYLOAD_DIR}\THIRD-PARTY-NOTICES.txt"
  SetOutPath "$INSTDIR\docs"
  File "${PAYLOAD_DIR}\docs\MANUAL-TESTS.md"
  File "${PAYLOAD_DIR}\docs\VALIDATION.md"
  File "${PAYLOAD_DIR}\docs\IMPLEMENTATION.md"
  File "${PAYLOAD_DIR}\docs\RELEASING.md"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Fullscreen Night Light"
  CreateShortcut "$SMPROGRAMS\Fullscreen Night Light\Fullscreen Night Light.lnk" "$INSTDIR\FullscreenNightLight.exe"
  CreateShortcut "$SMPROGRAMS\Fullscreen Night Light\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\FullscreenNightLight\Installer" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Fullscreen Night Light"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\FullscreenNightLight.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ; If this installation owned startup registration, keep its executable path current.
  ReadRegStr $0 HKCU "${RUN_KEY}" "FullscreenNightLight"
  ${If} $0 == '"$INSTDIR\FullscreenNightLight.exe" --startup'
    WriteRegStr HKCU "${RUN_KEY}" "FullscreenNightLight" '"$INSTDIR\FullscreenNightLight.exe" --startup'
  ${EndIf}
SectionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "Uninstall"
  ExecWait '"$INSTDIR\FullscreenNightLight.exe" --exit' $0
  Sleep 1000
  ClearErrors
  Delete "$INSTDIR\FullscreenNightLight.exe"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Close Fullscreen Night Light and retry uninstall. Its executable is still in use."
    Abort
  ${EndIf}
  ReadRegStr $0 HKCU "${RUN_KEY}" "FullscreenNightLight"
  ${If} $0 == '"$INSTDIR\FullscreenNightLight.exe" --startup'
    DeleteRegValue HKCU "${RUN_KEY}" "FullscreenNightLight"
  ${EndIf}
  ; Delete only known installed files. Preserve configuration, logs and unknown files.
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\config.example.toml"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\docs\MANUAL-TESTS.md"
  Delete "$INSTDIR\docs\VALIDATION.md"
  Delete "$INSTDIR\docs\IMPLEMENTATION.md"
  Delete "$INSTDIR\docs\RELEASING.md"
  RMDir "$INSTDIR\docs"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\Fullscreen Night Light\Fullscreen Night Light.lnk"
  Delete "$SMPROGRAMS\Fullscreen Night Light\Uninstall.lnk"
  RMDir "$SMPROGRAMS\Fullscreen Night Light"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\FullscreenNightLight\Installer"
  RMDir "$INSTDIR"
SectionEnd
