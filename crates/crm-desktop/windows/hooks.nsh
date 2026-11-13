; Хуки установщика NSIS (ТЗ §2.1.1). Установщик работает с правами администратора.

!macro NSIS_HOOK_PREINSTALL
  ; Обновление: служба держит crm-server.exe — останавливаем, чтобы заменить файл.
  nsExec::Exec '"$INSTDIR\crm-server.exe" service stop'
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Обновление: если служба была установлена — запускаем её снова.
  ; На чистой установке службы нет, команда просто завершится с ошибкой — это нормально.
  nsExec::Exec '"$INSTDIR\crm-server.exe" service start'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; При обновлении старый деинсталлятор запускается с /UPDATE — службу не трогаем.
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\crm-server.exe" service uninstall'
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Данные CRM удаляются только если пользователь сам отметил «Удалить данные приложения».
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext all
    RMDir /r "$COMMONAPPDATA\CRM"
  ${EndIf}
!macroend
