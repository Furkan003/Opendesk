; Opendesk Windows installer (NSIS).
;
; Build from the repository root after `cargo build --release --bins`:
;   makensis /DVERSION=0.2.0 /DBIN=target\release packaging\windows\opendesk.nsi
; Output: Opendesk-Windows-Kurulum.exe
;
; Run by an administrator (lab/IT setup) it installs for every user under Program Files
; and adds Windows Firewall rules for the classroom ports. Run by a normal user it
; installs only for that user, without administrator rights.
; Silent install for many computers: Opendesk-Windows-Kurulum.exe /S /ROLE=student

Unicode true
!include "MUI2.nsh"
!include "nsDialogs.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !define VERSION "0.2.0"
!endif
!ifndef BIN
  !define BIN "..\..\target\release"
!endif
!define APP "Opendesk"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Opendesk"

Name "${APP}"
OutFile "Opendesk-Windows-Kurulum.exe"
RequestExecutionLevel highest
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "Opendesk kurulumu"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Opendesk contributors (MIT)"

!define MUI_ICON "..\..\assets\opendesk-icon.ico"
!define MUI_UNICON "..\..\assets\opendesk-icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\opendesk.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Opendesk'i şimdi başlat"

Var Role
Var AllUsers
Var RadioPersonal
Var RadioTeacher
Var RadioStudent
Var CheckDesktop
Var CheckAutostart
Var WantDesktop
Var WantAutostart

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\..\LICENSE"
Page custom RolePage RolePageLeave
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Turkish"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  StrCpy $Role "personal"
  StrCpy $WantDesktop ${BST_CHECKED}
  StrCpy $WantAutostart ${BST_UNCHECKED}
  ; /ROLE=personal|teacher|student for silent installs.
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/ROLE=" $1
  ${IfNot} ${Errors}
    StrCpy $Role $1
    ${If} $Role == "student"
      StrCpy $WantAutostart ${BST_CHECKED}
    ${EndIf}
  ${EndIf}
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 == "Admin"
    StrCpy $AllUsers "1"
    SetShellVarContext all
    StrCpy $INSTDIR "$PROGRAMFILES64\${APP}"
  ${Else}
    StrCpy $AllUsers "0"
    SetShellVarContext current
    StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${APP}"
  ${EndIf}
FunctionEnd

Function RolePage
  !insertmacro MUI_HEADER_TEXT "Kullanım şekli" "Bu bilgisayar nasıl kullanılacak? Sonra Ayarlar'dan değiştirilebilir."
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateRadioButton} 0 0 100% 14u "Bireysel - uzak masaüstü (bağlan / ekranını paylaş)"
  Pop $RadioPersonal
  ${NSD_CreateRadioButton} 0 18u 100% 14u "Öğretmen - sınıf oluştur, öğrenci ekranlarını izle, ekranını yansıt"
  Pop $RadioTeacher
  ${NSD_CreateRadioButton} 0 36u 100% 14u "Öğrenci - öğretmenin sınıfına katıl"
  Pop $RadioStudent
  ${If} $Role == "teacher"
    ${NSD_Check} $RadioTeacher
  ${ElseIf} $Role == "student"
    ${NSD_Check} $RadioStudent
  ${Else}
    ${NSD_Check} $RadioPersonal
  ${EndIf}
  ${NSD_CreateCheckBox} 0 64u 100% 12u "Masaüstü kısayolu oluştur"
  Pop $CheckDesktop
  ${NSD_Check} $CheckDesktop
  ${NSD_CreateCheckBox} 0 80u 100% 12u "Windows açılınca Opendesk'i başlat (öğrenci bilgisayarları için önerilir)"
  Pop $CheckAutostart
  ${If} $WantAutostart == ${BST_CHECKED}
    ${NSD_Check} $CheckAutostart
  ${EndIf}
  ${NSD_CreateLabel} 0 98u 100% 24u "Otomatik başlatma yalnızca pencereyi açar; ekran paylaşımı veya sınıfa katılım kullanıcının seçimiyle olur."
  Pop $0
  nsDialogs::Show
FunctionEnd

Function RolePageLeave
  ${NSD_GetState} $RadioTeacher $0
  ${NSD_GetState} $RadioStudent $1
  ${If} $0 == ${BST_CHECKED}
    StrCpy $Role "teacher"
  ${ElseIf} $1 == ${BST_CHECKED}
    StrCpy $Role "student"
  ${Else}
    StrCpy $Role "personal"
  ${EndIf}
  ${NSD_GetState} $CheckDesktop $WantDesktop
  ${NSD_GetState} $CheckAutostart $WantAutostart
FunctionEnd

Section "Opendesk" Main
  SectionIn RO
  ; An open Opendesk keeps its executable locked; close it before copying.
  nsExec::Exec 'taskkill /IM opendesk.exe /F'
  SetOutPath "$INSTDIR"
  File "${BIN}\opendesk.exe"
  File "${BIN}\opendesk-relay.exe"
  File "${BIN}\opendesk-updater.exe"
  File "..\..\LICENSE"
  FileOpen $0 "$INSTDIR\role.txt" w
  FileWrite $0 $Role
  FileClose $0
  WriteUninstaller "$INSTDIR\Kaldir.exe"

  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\opendesk.exe"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP} kaldır.lnk" "$INSTDIR\Kaldir.exe"
  ${If} $WantDesktop == ${BST_CHECKED}
    CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\opendesk.exe"
  ${EndIf}
  ${If} $WantAutostart == ${BST_CHECKED}
    CreateShortcut "$SMSTARTUP\${APP}.lnk" "$INSTDIR\opendesk.exe"
  ${EndIf}

  ${If} $AllUsers == "1"
    ; Teacher: class port (TCP 7880) and remote desktop (TCP 7878).
    ; Student: class discovery beacon (UDP 7881) and remote desktop (TCP 7878).
    nsExec::Exec 'netsh advfirewall firewall delete rule name="Opendesk"'
    nsExec::Exec 'netsh advfirewall firewall add rule name="Opendesk" dir=in action=allow program="$INSTDIR\opendesk.exe" enable=yes profile=private,domain'
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${APP}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "Opendesk contributors"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\opendesk.exe"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Kaldir.exe"'
    WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Kaldir.exe" /S'
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
  ${Else}
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Opendesk contributors"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\opendesk.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Kaldir.exe"'
    WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Kaldir.exe" /S'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${EndIf}
SectionEnd

Function un.onInit
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 == "Admin"
  ${AndIf} ${FileExists} "$PROGRAMFILES64\${APP}\Kaldir.exe"
    StrCpy $AllUsers "1"
    SetShellVarContext all
  ${Else}
    StrCpy $AllUsers "0"
    SetShellVarContext current
  ${EndIf}
FunctionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM opendesk.exe /F'
  Delete "$INSTDIR\opendesk.exe"
  Delete "$INSTDIR\opendesk-relay.exe"
  Delete "$INSTDIR\opendesk-updater.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\role.txt"
  Delete "$INSTDIR\Kaldir.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}\${APP}.lnk"
  Delete "$SMPROGRAMS\${APP}\${APP} kaldır.lnk"
  RMDir "$SMPROGRAMS\${APP}"
  Delete "$DESKTOP\${APP}.lnk"
  Delete "$SMSTARTUP\${APP}.lnk"
  ${If} $AllUsers == "1"
    nsExec::Exec 'netsh advfirewall firewall delete rule name="Opendesk"'
    DeleteRegKey HKLM "${UNINSTALL_KEY}"
  ${Else}
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ${EndIf}
SectionEnd
