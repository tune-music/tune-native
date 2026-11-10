; Установщик Tune Native: для текущего пользователя, без прав администратора.
; Собирается скриптом installer/build.ps1, он же передаёт версию и пути.
;
; Ключи: /S — без окон (так ставит обновления сам Tune), /R — запустить Tune после установки.
; Запущенный Tune установщик не закрывает: ждёт, пока его закроют. При обновлении Tune
; закрывается сам, сразу после запуска установщика.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "Нужна версия: /DVERSION=0.1.0"
!endif
!ifndef RELEASE
  !error "Нужна папка сборки: /DRELEASE=..\target\release"
!endif
!ifndef OUTFILE
  !define OUTFILE "Tune-Native-${VERSION}-setup.exe"
!endif

!define NAME "Tune Native"
!define EXE "tune-native.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\TuneNative"
; Ярлык в «Пуске» с тем же именем ведёт и сама программа (shell.rs): ставит на него свой
; идентификатор, чтобы панель задач узнавала Tune.
!define SHORTCUT "Tune.lnk"
!define DATA "$APPDATA\dev.tune.native"

Name "${NAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Tune Native"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "Tune Native ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1049 "ProductName" "Tune Native"
VIAddVersionKey /LANG=1049 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1049 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1049 "FileDescription" "Установка Tune Native"
VIAddVersionKey /LANG=1049 "CompanyName" "Tune"
VIAddVersionKey /LANG=1049 "LegalCopyright" "Tune"

!define MUI_ICON "..\assets\tune.ico"
!define MUI_UNICON "..\assets\tune.ico"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_BITMAP "header.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "header.bmp"
!define MUI_WELCOMEFINISHPAGE_BITMAP "sidebar.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "sidebar.bmp"
!define MUI_ABORTWARNING

!define MUI_WELCOMEPAGE_TITLE "Установка Tune Native"
!define MUI_WELCOMEPAGE_TEXT "Tune Native — клиент Яндекс Музыки с нативным интерфейсом.$\r$\n$\r$\nTune ставится только для вас, права администратора не нужны. Настройки и вход в аккаунт сохранятся, если Tune уже был у вас.$\r$\n$\r$\nНажмите «Далее», чтобы продолжить."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Запустить Tune"
; Вторая галочка страницы — ярлык на рабочем столе.
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Ярлык на рабочем столе"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION DesktopShortcut
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "Russian"

Var Relaunch

; Ждёт, пока Tune закроется: запущенный exe нельзя открыть на запись.
; Тихо — до 30 секунд, с окном — спрашивает.
!macro WaitClosed
  StrCpy $1 0
  ${Do}
    ${IfNot} ${FileExists} "$INSTDIR\${EXE}"
      ${Break}
    ${EndIf}
    ClearErrors
    FileOpen $0 "$INSTDIR\${EXE}" a
    ${IfNot} ${Errors}
      FileClose $0
      ${Break}
    ${EndIf}
    ${If} ${Silent}
      IntOp $1 $1 + 1
      ${If} $1 > 60
        Abort "Tune не закрылся"
      ${EndIf}
      Sleep 500
    ${Else}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Tune сейчас открыт. Закройте его (и в трее тоже) и нажмите «Повтор»." IDRETRY wait_retry
      Abort "Tune открыт"
      wait_retry:
    ${EndIf}
  ${Loop}
!macroend

Function .onInit
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/R" $R1
  ${IfNot} ${Errors}
    StrCpy $Relaunch 1
  ${EndIf}
FunctionEnd

Function DesktopShortcut
  CreateShortcut "$DESKTOP\Tune.lnk" "$INSTDIR\${EXE}"
FunctionEnd

Section "Tune Native" Main
  SetOutPath "$INSTDIR"
  !insertmacro WaitClosed
  File "${RELEASE}\${EXE}"
  ; Видеоплеер для клипов и живого фона тем. Без него клипы открываются в браузере.
  File /nonfatal "${RELEASE}\libmpv-2.dll"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${SHORTCUT}" "$INSTDIR\${EXE}"
  ; Программа сверяет ярлык по этой отметке; без неё она заново ставит на ярлык свой идентификатор.
  Delete "${DATA}\shortcut.txt"

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Tune Native"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Tune"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/tune-music/tune-native"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLUpdateInfo" "https://github.com/tune-music/tune-native/releases"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"

  ${If} $Relaunch == 1
    Exec '"$INSTDIR\${EXE}"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  !insertmacro WaitClosed
  ; Tune возвращает ссылки yandexmusic:// прежнему обработчику и убирает автозапуск.
  ExecWait '"$INSTDIR\${EXE}" --uninstall'
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\libmpv-2.dll"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${SHORTCUT}"
  Delete "$DESKTOP\Tune.lnk"
  Delete "${DATA}\shortcut.txt"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
