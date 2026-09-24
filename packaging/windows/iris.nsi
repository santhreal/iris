; ==============================================================================
; iris Windows Installer (NSIS Script)
;
; References:
;   packaging/CONTRACT.md
;
; Identity:
;   - App Name:            iris
;   - App / Bundle ID:     dev.iris.app
;   - Executable:          iris.exe
;   - Install Scope:       Per-user (no administrator privileges required)
;   - Install Directory:   %LOCALAPPDATA%\Programs\iris
;   - Start Menu Shortcut: iris.lnk -> iris.exe --home
;   - Autostart:           HKCU\Software\Microsoft\Windows\CurrentVersion\Run
;                          Value 'iris' -> "$INSTDIR\iris.exe" --daemon
;   - Uninstaller:         $INSTDIR\uninstall.exe
;                          Removes files, shortcuts, autostart Run entry, and
;                          Add/Remove Programs registration.
;
; Runtime Model:
;   - iris (no args) = background daemon (tray + global hotkeys + IPC listener),
;                      or the home window of the daemon that runs.
;   - iris --daemon  = background daemon; exits when a daemon runs.
;   - iris --home    = opens/surfaces the home window in the daemon.
;   - iris --quit    = asks the running daemon to shut down gracefully.
;
; Supported Parameters:
;   - /S             = Silent installation / uninstallation.
;   - /RUN           = Start iris when the installation ends, whether it
;                      succeeded or failed. `iris --update` starts the
;                      installer with /S /RUN.
;   - /D=path        = Custom target directory (must be the last parameter).
; ==============================================================================

Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

; ------------------------------------------------------------------------------
; Build Configuration & Default Variables
; (Can be overridden on the command line via -DVAR=VAL)
; ------------------------------------------------------------------------------
!ifndef VERSION
  !define VERSION "0.1.0"
!endif

!ifndef VERSION_QUAD
  !define VERSION_QUAD "${VERSION}.0"
!endif

!ifndef APP_NAME
  !define APP_NAME "iris"
!endif

!ifndef APP_ID
  !define APP_ID "dev.iris.app"
!endif

!ifndef PUBLISHER
  !define PUBLISHER "Santh"
!endif

!ifndef URL
  !define URL "https://github.com/santhreal/iris"
!endif

!ifndef DESCRIPTION
  !define DESCRIPTION "Screenshot and screen-recording utility"
!endif

!ifndef BINARY_PATH
  !define BINARY_PATH "..\..\target\release\iris.exe"
!endif

!ifndef ICON_PATH
  !define ICON_PATH "..\icons\iris.ico"
!endif

!ifndef OUTFILE
  !define OUTFILE "..\..\dist\iris-${VERSION}-windows-x86_64-setup.exe"
!endif

; ------------------------------------------------------------------------------
; General Settings
; ------------------------------------------------------------------------------
Name "${APP_NAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
InstallDirRegKey HKCU "Software\${APP_ID}" "InstallDir"
BrandingText "${APP_NAME} ${VERSION}"

; ------------------------------------------------------------------------------
; Version & Executable Metadata
; ------------------------------------------------------------------------------
VIProductVersion "${VERSION_QUAD}"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP_NAME} - ${DESCRIPTION}"
VIAddVersionKey "FileVersion" "${VERSION_QUAD}"
VIAddVersionKey "InternalName" "iris-setup"
VIAddVersionKey "LegalCopyright" "${PUBLISHER}"
VIAddVersionKey "OriginalFilename" "iris-${VERSION}-windows-x86_64-setup.exe"

; ------------------------------------------------------------------------------
; Modern UI (MUI2), LogicLib, and FileFunc Includes
; ------------------------------------------------------------------------------
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; Registry key constants
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"
!define APP_KEY "Software\${APP_ID}"

; Icon settings
!define MUI_ICON "${ICON_PATH}"
!define MUI_UNICON "${ICON_PATH}"
!define MUI_ABORTWARNING

; ------------------------------------------------------------------------------
; Installer Pages
; ------------------------------------------------------------------------------
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES

; Finish page with optional launch of iris
!define MUI_FINISHPAGE_RUN "$INSTDIR\iris.exe"
!define MUI_FINISHPAGE_RUN_PARAMETERS "--home"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${APP_NAME}"
!insertmacro MUI_PAGE_FINISH

; ------------------------------------------------------------------------------
; Uninstaller Pages
; ------------------------------------------------------------------------------
!insertmacro MUI_UNPAGE_WELCOME
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

; ------------------------------------------------------------------------------
; Language
; ------------------------------------------------------------------------------
!insertmacro MUI_LANGUAGE "English"

; ------------------------------------------------------------------------------
; Stopping iris
; ------------------------------------------------------------------------------
; Ask a running daemon to quit, then wait up to 30 seconds until no
; process runs $INSTDIR\iris.exe: Windows denies write access to the
; file of a running program. While iris.exe stays in use, a message box
; offers Retry, which waits again, and Cancel, which aborts. A silent
; run aborts.
!macro StopIris
    ${If} ${FileExists} "$INSTDIR\iris.exe"
        DetailPrint "Stopping ${APP_NAME}..."
        ExecWait '"$INSTDIR\iris.exe" --quit'
        ${Do}
            StrCpy $R1 0
            ${Do}
                ClearErrors
                FileOpen $R0 "$INSTDIR\iris.exe" a
                ${IfNot} ${Errors}
                    FileClose $R0
                    StrCpy $R1 "free"
                    ${Break}
                ${EndIf}
                IntOp $R1 $R1 + 1
                ${If} $R1 >= 300
                    ${Break}
                ${EndIf}
                Sleep 100
            ${Loop}
            ${If} $R1 == "free"
                ${Break}
            ${EndIf}
            ${IfNot} ${Cmd} `MessageBox MB_RETRYCANCEL|MB_ICONSTOP "$INSTDIR\iris.exe is in use. Quit iris, then click Retry." /SD IDCANCEL IDRETRY`
                Abort
            ${EndIf}
        ${Loop}
    ${EndIf}
!macroend

; ------------------------------------------------------------------------------
; Installer Initialization
; ------------------------------------------------------------------------------
Function .onInit
    ; Default $INSTDIR if not set by /D= or previous registry entry
    ${If} $INSTDIR == ""
        StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${APP_NAME}"
    ${EndIf}
FunctionEnd

; With /RUN, start $INSTDIR\iris.exe: the new one after a successful
; installation, the one found in place after a failed one. It starts with
; no option: an iris.exe found in place may predate --daemon.
Function StartIfRequested
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "/RUN" $R1
    ${IfNot} ${Errors}
        Exec '"$INSTDIR\iris.exe"'
    ${EndIf}
FunctionEnd

Function .onInstFailed
    Call StartIfRequested
FunctionEnd

; ------------------------------------------------------------------------------
; Installation Section
; ------------------------------------------------------------------------------
Section "Install" SecInstall
    ; 1. Stop a running iris before replacing its executable
    !insertmacro StopIris

    ; 2. Create destination directory and copy files
    SetOutPath "$INSTDIR"
    DetailPrint "Installing ${APP_NAME} files..."
    File /oname=iris.exe "${BINARY_PATH}"
    File /oname=iris.ico "${ICON_PATH}"

    ; 3. Generate uninstaller
    DetailPrint "Creating uninstaller..."
    WriteUninstaller "$INSTDIR\uninstall.exe"

    ; 4. Create Start Menu shortcut: 'iris' running 'iris.exe --home'
    DetailPrint "Creating Start Menu shortcut..."
    SetOutPath "$INSTDIR"
    CreateShortcut "$SMPROGRAMS\iris.lnk" "$INSTDIR\iris.exe" "--home" "$INSTDIR\iris.ico" 0 SW_SHOWNORMAL "" "${APP_NAME} - ${DESCRIPTION}"

    ; 5. Register HKCU\...\Run autostart: runs the daemon at login, and
    ;    nothing when a daemon already runs
    DetailPrint "Registering login autostart..."
    WriteRegStr HKCU "${RUN_KEY}" "iris" '"$INSTDIR\iris.exe" --daemon'

    ; 6. Store app metadata in registry for update checks and location lookup
    WriteRegStr HKCU "${APP_KEY}" "InstallDir" "$INSTDIR"
    WriteRegStr HKCU "${APP_KEY}" "Version" "${VERSION}"

    ; 7. Register in Windows Add/Remove Programs (Programs and Features)
    DetailPrint "Registering Add/Remove Programs entry..."
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" '"$INSTDIR\iris.ico"'
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
    WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "${URL}"
    WriteRegStr HKCU "${UNINST_KEY}" "HelpLink" "${URL}"
    WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

    ; Compute estimated size in KB for Add/Remove Programs
    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    IntFmt $0 "0x%08X" $0
    WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0

    ; 8. /RUN: start the installed iris
    Call StartIfRequested
SectionEnd

; ------------------------------------------------------------------------------
; Uninstallation Section
; ------------------------------------------------------------------------------
Section "Uninstall"
    ; 1. Stop a running iris before deleting its executable
    !insertmacro StopIris

    ; 2. Remove autostart entry from HKCU\...\Run
    DetailPrint "Removing login autostart entry..."
    DeleteRegValue HKCU "${RUN_KEY}" "iris"

    ; 3. Remove Start Menu shortcut
    DetailPrint "Removing Start Menu shortcut..."
    Delete "$SMPROGRAMS\iris.lnk"

    ; 4. Remove installed files
    DetailPrint "Removing application files..."
    Delete "$INSTDIR\iris.exe"
    Delete "$INSTDIR\iris.ico"
    Delete "$INSTDIR\uninstall.exe"

    ; 5. Remove registry entries
    DetailPrint "Cleaning registry entries..."
    DeleteRegKey HKCU "${UNINST_KEY}"
    DeleteRegKey HKCU "${APP_KEY}"

    ; 6. Remove installation directory if empty
    DetailPrint "Removing installation directory..."
    RMDir "$INSTDIR"
    ; Attempt to clean up Programs directory if empty (safe no-op if other apps exist)
    RMDir "$LOCALAPPDATA\Programs"
SectionEnd
