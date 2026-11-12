; Qust's Windows installer (NSIS 3), for the native app: one executable, nothing else.
;
; Derived from the installer.nsi that Tauri's bundler (tauri-bundler 2, NSIS templates; Copyright
; 2019-2025 Tauri Programme within The Commons Conservancy, licensed Apache-2.0 OR MIT) generated
; for Qust 1.1.x, and changed for Qust (GPL-3.0-or-later). Run over a Tauri install, it upgrades it
; in place, because it keeps everything that install is known by:
;   - per-user, in %LOCALAPPDATA%\Qust (or wherever HKCU\Software\pascalmuangkhot\Qust says)
;   - the executable qust.exe, and uninstall.exe next to it (which the app takes to mean
;     "installed": only then does it update itself)
;   - HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Qust, with the same values
;   - the Start menu shortcut Qust.lnk (AppUserModelId com.pascalmuangkhot.qust), pins included
;   - the startup entry HKCU\...\Run "Qust" = "<dir>\qust.exe --background", which it leaves alone
;     (the path doesn't change); the uninstaller removes it, as Tauri's did
;   - the command line of both updaters (Tauri's and qust-updater): /P passive, /S silent,
;     /UPDATE no uninstall and no new shortcuts, /R relaunch, /ARGS the relaunch arguments,
;     /NS no shortcuts, /D=<dir>
; It closes Qust first, even a tray-only instance, through the Restart Manager.
; What it drops: the WebView2 bootstrap (no webview any more), WiX migration (there never was an
; MSI), the language selector and the nsis_tauri_utils plugin, so a stock NSIS builds it. On an
; upgrade it deletes the WebView2 cache the Tauri version left (%LOCALAPPDATA%\<id>\EBWebView).
;
; Build it with packaging/windows/build-installer.mjs, which passes:
;   /DVERSION=1.2.0 /DVERSIONWITHBUILD=1.2.0.0 /DMAINBINARYSRCPATH=<exe> /DOUTFILE=<setup.exe>
;   [/DINSTALLERICON=<ico>]
; PRODUCTNAME, MANUFACTURER, BUNDLEID and MAINBINARYNAME can be overridden too, only to build a
; test installer that can't touch the real install.

Unicode true
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2
SetCompressor /SOLID "lzma"

!include MUI2.nsh
!include LogicLib.nsh
!include nsDialogs.nsh
!include FileFunc.nsh
!include x64.nsh
!include WordFunc.nsh
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "Win\RestartManager.nsh"
!include "utils.nsh"

!ifndef VERSION
  !error "Pass /DVERSION=x.y.z"
!endif
!ifndef VERSIONWITHBUILD
  !error "Pass /DVERSIONWITHBUILD=x.y.z.0"
!endif
!ifndef MAINBINARYSRCPATH
  !error "Pass /DMAINBINARYSRCPATH=<path to the app's exe>"
!endif
!ifndef OUTFILE
  !error "Pass /DOUTFILE=<installer to write>"
!endif
!define /ifndef PRODUCTNAME "Qust"
!define /ifndef MANUFACTURER "pascalmuangkhot"
!define /ifndef BUNDLEID "com.pascalmuangkhot.qust"
!define /ifndef MAINBINARYNAME "qust"
!define /ifndef HOMEPAGE "https://github.com/mkpascal/qust"
!define /ifndef COPYRIGHT ""

!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUKEY "Software\${MANUFACTURER}"
!define MANUPRODUCTKEY "${MANUKEY}\${PRODUCTNAME}"
!define RUNKEY "Software\Microsoft\Windows\CurrentVersion\Run"

Var PassiveMode
Var UpdateMode
Var NoShortcutMode
Var OldMainBinaryName

Name "${PRODUCTNAME}"
BrandingText "${COPYRIGHT}"
OutFile "${OUTFILE}"
RequestExecutionLevel user

; Not the default install path, only what makes the directory page add the product folder:
; .onInit sets the real one.
!define PLACEHOLDER_INSTALL_DIR "placeholder\${PRODUCTNAME}"
InstallDir "${PLACEHOLDER_INSTALL_DIR}"

VIProductVersion "${VERSIONWITHBUILD}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!ifdef INSTALLERICON
  !define MUI_ICON "${INSTALLERICON}"
  !define MUI_UNICON "${INSTALLERICON}"
!endif

; Where the installer language is kept, as before
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${MANUPRODUCTKEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Installer Language"

; Installer pages
; 1. Welcome
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_WELCOME

; 2. An installation is there already: reinstall/upgrade it in place, or uninstall it first
Var ReinstallPageCheck
Page custom PageReinstall PageLeaveReinstall

Function PageReinstall
  ReadRegStr $R0 SHCTX "${UNINSTKEY}" ""
  ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"
  ${IfThen} "$R0$R1" == "" ${|} Abort ${|}

  ; $R0: 0 same version, 1 upgrading, -1 downgrading
  ReadRegStr $R0 SHCTX "${UNINSTKEY}" "DisplayVersion"
  StrCpy $R4 "$(older)"
  ${IfThen} $R0 == "" ${|} StrCpy $R4 "$(unknown)" ${|}
  Push $R0
  Call NumericVersion
  Pop $R0
  ${VersionCompare} "${VERSIONWITHBUILD}" "$R0.0" $R0
  ${If} $R0 = 2
    StrCpy $R0 -1
  ${EndIf}

  ${If} $R0 = 0
    StrCpy $R1 "$(alreadyInstalledLong)"
    StrCpy $R2 "$(addOrReinstall)"
    StrCpy $R3 "$(uninstallApp)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(chooseMaintenanceOption)"
  ${ElseIf} $R0 = 1
    StrCpy $R1 "$(olderOrUnknownVersionInstalled)"
    StrCpy $R2 "$(upgradeInPlace)"
    StrCpy $R3 "$(uninstallBeforeInstalling)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(chooseHowToInstall)"
  ${Else}
    StrCpy $R1 "$(newerVersionInstalled)"
    StrCpy $R2 "$(upgradeInPlace)"
    StrCpy $R3 "$(uninstallBeforeInstalling)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(chooseHowToInstall)"
  ${EndIf}

  ; Passive, silent and update installs skip the page, and so install in place
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
  ${OrIf} $UpdateMode = 1
    StrCpy $ReinstallPageCheck 1
    Call PageLeaveReinstall
  ${Else}
    nsDialogs::Create 1018
    Pop $R4
    ${IfThen} $(^RTL) = 1 ${|} nsDialogs::SetRTL $(^RTL) ${|}

    ${NSD_CreateLabel} 0 0 100% 24u $R1
    Pop $R1

    ${NSD_CreateRadioButton} 30u 50u -30u 8u $R2
    Pop $R2
    ${NSD_OnClick} $R2 PageReinstallUpdateSelection

    ${NSD_CreateRadioButton} 30u 70u -30u 8u $R3
    Pop $R3
    ${NSD_OnClick} $R3 PageReinstallUpdateSelection

    ; The first choice (keep: reinstall or upgrade in place) unless the second was picked before.
    ; Uninstalling first would also remove the startup entry and unpin Qust.
    ${If} $ReinstallPageCheck <> 2
      StrCpy $ReinstallPageCheck 1
      SendMessage $R2 ${BM_SETCHECK} ${BST_CHECKED} 0
    ${Else}
      SendMessage $R3 ${BM_SETCHECK} ${BST_CHECKED} 0
    ${EndIf}

    ${NSD_SetFocus} $R2
    nsDialogs::Show
  ${EndIf}
FunctionEnd

Function PageReinstallUpdateSelection
  ${NSD_GetState} $R2 $R1
  ${If} $R1 == ${BST_CHECKED}
    StrCpy $ReinstallPageCheck 1
  ${Else}
    StrCpy $ReinstallPageCheck 2
  ${EndIf}
FunctionEnd

Function PageLeaveReinstall
  ; Updates never uninstall first
  ${If} $UpdateMode = 1
  ${OrIf} $ReinstallPageCheck <> 2
    Goto reinst_done
  ${EndIf}

  HideWindow
  ClearErrors
  ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
  ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"
  ${IfThen} $PassiveMode = 1 ${|} StrCpy $R1 "$R1 /P" ${|}
  StrCpy $R1 "$R1 _?=$4" ; run it in place, from the install directory
  ExecWait '$R1' $0
  BringToFront

  ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed
  ${If} $0 <> 0
  ${OrIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    ; Cancelled: back to the choice
    ${If} $0 = 1
      Abort
    ${EndIf}
    MessageBox MB_ICONEXCLAMATION "$(unableToUninstall)"
    Abort
  ${EndIf}

  ; The uninstaller ran in place (`_?=`), so it couldn't delete itself and the directory
  ${If} $4 != ""
    Delete "$4\uninstall.exe"
    RMDir "$4"
  ${EndIf}
  reinst_done:
FunctionEnd

; "1.2.3-beta+5" -> "1.2.3", for VersionCompare, which knows only numbers and dots
Function NumericVersion
  Exch $0
  Push $1
  Push $2
  StrCpy $1 0
  loop:
    StrCpy $2 $0 1 $1
    StrCmp $2 "" done
    StrCmp $2 "-" cut
    StrCmp $2 "+" cut
    IntOp $1 $1 + 1
    Goto loop
  cut:
    StrCpy $0 $0 $1
  done:
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; 3. Install directory
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_DIRECTORY

; 4. Installing
!insertmacro MUI_PAGE_INSTFILES

; 5. Finish: the "show readme" checkbox makes the desktop shortcut, the "run" one starts Qust
!define MUI_FINISHPAGE_NOAUTOCLOSE
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(createDesktop)"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateOrUpdateDesktopShortcut
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

Function RunMainBinary
  Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
FunctionEnd

; Uninstaller pages
; 1. Confirm, with a "Delete the application data" checkbox
Var DeleteAppDataCheckbox
Var DeleteAppDataCheckboxState
!define /ifndef WS_EX_LAYOUTRTL 0x00400000
!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.ConfirmShow
Function un.ConfirmShow
  ; $1 inner dialog HWND, $2 window DPI, $3 style, $4 x, $5 y, $6 width, $7 height
  FindWindow $1 "#32770" "" $HWNDPARENT
  System::Call "user32::GetDpiForWindow(p r1) i .r2"
  ${If} $(^RTL) = 1
    StrCpy $3 "${__NSD_CheckBox_EXSTYLE} | ${WS_EX_LAYOUTRTL}"
    IntOp $4 50 * $2
  ${Else}
    StrCpy $3 "${__NSD_CheckBox_EXSTYLE}"
    IntOp $4 0 * $2
  ${EndIf}
  IntOp $5 100 * $2
  IntOp $6 400 * $2
  IntOp $7 25 * $2
  IntOp $4 $4 / 96
  IntOp $5 $5 / 96
  IntOp $6 $6 / 96
  IntOp $7 $7 / 96
  System::Call 'user32::CreateWindowEx(i r3, w "${__NSD_CheckBox_CLASS}", w "$(deleteAppData)", i ${__NSD_CheckBox_STYLE}, i r4, i r5, i r6, i r7, p r1, i0, i0, i0) i .s'
  Pop $DeleteAppDataCheckbox
  SendMessage $HWNDPARENT ${WM_GETFONT} 0 0 $1
  SendMessage $DeleteAppDataCheckbox ${WM_SETFONT} $1 1
FunctionEnd
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave
Function un.ConfirmLeave
  SendMessage $DeleteAppDataCheckbox ${BM_GETCHECK} 0 0 $DeleteAppDataCheckboxState
FunctionEnd
!define MUI_PAGE_CUSTOMFUNCTION_PRE un.SkipIfPassive
!insertmacro MUI_UNPAGE_CONFIRM

; 2. Uninstalling
!insertmacro MUI_UNPAGE_INSTFILES

; Languages
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_RESERVEFILE_LANGDLL

LangString addOrReinstall ${LANG_ENGLISH} "Add/Reinstall components"
LangString alreadyInstalled ${LANG_ENGLISH} "Already Installed"
LangString alreadyInstalledLong ${LANG_ENGLISH} "${PRODUCTNAME} ${VERSION} is already installed. Select the operation you want to perform and click Next to continue."
LangString appRunning ${LANG_ENGLISH} "${PRODUCTNAME} is running! Please close it first then try again."
LangString appRunningOkKill ${LANG_ENGLISH} "${PRODUCTNAME} is running!$\nClick OK to close it"
LangString chooseMaintenanceOption ${LANG_ENGLISH} "Choose the maintenance option to perform."
LangString chooseHowToInstall ${LANG_ENGLISH} "Choose how you want to install ${PRODUCTNAME}."
LangString createDesktop ${LANG_ENGLISH} "Create desktop shortcut"
LangString failedToKillApp ${LANG_ENGLISH} "Failed to close ${PRODUCTNAME}. Please close it first then try again"
LangString newerVersionInstalled ${LANG_ENGLISH} "A newer version of ${PRODUCTNAME} is already installed. Select the operation you want to perform and click Next to continue."
LangString older ${LANG_ENGLISH} "older"
LangString olderOrUnknownVersionInstalled ${LANG_ENGLISH} "An $R4 version of ${PRODUCTNAME} is installed on your system. Select the operation you want to perform and click Next to continue."
LangString unableToUninstall ${LANG_ENGLISH} "Unable to uninstall!"
LangString uninstallApp ${LANG_ENGLISH} "Uninstall ${PRODUCTNAME}"
LangString uninstallBeforeInstalling ${LANG_ENGLISH} "Uninstall before installing (also removes the startup entry and pins)"
LangString upgradeInPlace ${LANG_ENGLISH} "Install over it, keeping settings, shortcuts and the startup entry"
LangString unknown ${LANG_ENGLISH} "unknown"
LangString deleteAppData ${LANG_ENGLISH} "Delete the application data"

Function .onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/NS" $NoShortcutMode
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}

  !insertmacro SetContext

  ${If} $INSTDIR == "${PLACEHOLDER_INSTALL_DIR}"
    StrCpy $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}"
    Call RestorePreviousInstallLocation
  ${EndIf}
FunctionEnd

Section Install
  SetOutPath $INSTDIR

  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro WaitUntilWritable "$INSTDIR\${MAINBINARYNAME}.exe"

  ; The app, under its fixed name whatever the built file is called
  File "/oname=${MAINBINARYNAME}.exe" "${MAINBINARYSRCPATH}"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Where to install next time
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR

  ; An older install under another executable name: remove that one
  ReadRegStr $OldMainBinaryName SHCTX "${UNINSTKEY}" "MainBinaryName"
  ${If} $OldMainBinaryName != ""
  ${AndIf} $OldMainBinaryName != "${MAINBINARYNAME}.exe"
    Delete "$INSTDIR\$OldMainBinaryName"
  ${EndIf}
  WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"

  ; Apps & features
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\"$INSTDIR$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" "1"
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" "1"
  WriteRegStr SHCTX "${UNINSTKEY}" "URLInfoAbout" "${HOMEPAGE}"
  ${GetSize} "$INSTDIR" "/S=0K /G=0" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "$0"

  Call CreateOrUpdateStartMenuShortcut

  ; Silent and passive installs skip the finish page and its checkbox
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Call CreateOrUpdateDesktopShortcut
  ${EndIf}

  ; The Tauri version's WebView2 profile, which nothing reads any more. Settings are elsewhere
  ; (%APPDATA%\${BUNDLEID}\settings.json) and stay.
  RMDir /r "$LOCALAPPDATA\${BUNDLEID}\EBWebView"
  RMDir "$LOCALAPPDATA\${BUNDLEID}"

  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function .onInstSuccess
  ; /R only for silent and passive installs: the finish page has its own "run" checkbox
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $R0
    ${IfNot} ${Errors}
      ${GetOptions} $CMDLINE "/ARGS" $R0
      SetOutPath $INSTDIR
      Exec '"$INSTDIR\${MAINBINARYNAME}.exe" $R0'
    ${EndIf}
  ${EndIf}
FunctionEnd

Function un.onInit
  !insertmacro SetContext
  !insertmacro MUI_UNGETLANGUAGE

  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
FunctionEnd

Section Uninstall
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe"

  Delete "$INSTDIR\${MAINBINARYNAME}.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; Shortcuts, unless updating
  ${If} $UpdateMode <> 1
    !insertmacro DeleteAppUserModelId

    !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${EndIf}

    !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$DESKTOP\${PRODUCTNAME}.lnk"
      Delete "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
  ${EndIf}

  DeleteRegKey HKCU "${UNINSTKEY}"

  ; The startup entry the app writes ("Background helper"), unless updating
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "${RUNKEY}" "${PRODUCTNAME}"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"
  ${EndIf}

  ; App data, when asked and not updating
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"
    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
    DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
    SetShellVarContext current
    RMDir /r "$APPDATA\${BUNDLEID}"
    RMDir /r "$LOCALAPPDATA\${BUNDLEID}"
  ${EndIf}

  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function RestorePreviousInstallLocation
  ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
  StrCmp $4 "" +2 0
    StrCpy $INSTDIR $4
FunctionEnd

Function SkipIfPassive
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
FunctionEnd
Function un.SkipIfPassive
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
FunctionEnd

Function CreateOrUpdateStartMenuShortcut
  ; A shortcut to an old executable name: point it at the new one
  ${If} $OldMainBinaryName != ""
    !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
    Pop $0
    ${If} $0 = 1
      !insertmacro SetShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      Return
    ${EndIf}
  ${EndIf}

  ; Updates keep the shortcuts as they are (the user may have removed them)
  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    Return
  ${EndIf}

  CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
FunctionEnd

Function CreateOrUpdateDesktopShortcut
  ${If} $OldMainBinaryName != ""
    !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
    Pop $0
    ${If} $0 = 1
      !insertmacro SetShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      Return
    ${EndIf}
  ${EndIf}

  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    Return
  ${EndIf}

  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
FunctionEnd
