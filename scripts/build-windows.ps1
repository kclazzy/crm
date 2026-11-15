# Сборка установщика CRM на своём компьютере с Windows — те же шаги, что в CI.
#
# Запуск из папки проекта (PowerShell):
#   powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
#
# Нужны: Rust (rustup), Visual Studio Build Tools с «Разработка классических
# приложений на C++», Node.js 22 LTS. Проверка — ниже, с подсказкой, что поставить.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Need($cmd, $hint) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Write-Host "Не найдено: $cmd" -ForegroundColor Red
        Write-Host "Установите: $hint" -ForegroundColor Yellow
        exit 1
    }
}
Need cargo 'winget install Rustlang.Rustup   (затем перезапустите PowerShell)'
Need node  'winget install OpenJS.NodeJS.LTS  (затем перезапустите PowerShell)'
Need npm   'winget install OpenJS.NodeJS.LTS'

function Step($text) { Write-Host "`n=== $text ===" -ForegroundColor Cyan }
function Run($what) {
    & $what
    if ($LASTEXITCODE) { throw "ошибка на шаге выше (код $LASTEXITCODE)" }
}

Step '1/4 Веб-интерфейс'
Push-Location web
Run { npm ci }
Run { npm run build }
Pop-Location

Step '2/4 Сервер CRM (crm-server)'
Run { cargo build --release -p crm-server }

Step '3/4 Сервер рядом с программой'
New-Item -ItemType Directory -Force crates\crm-desktop\binaries | Out-Null
Copy-Item target\release\crm-server.exe crates\crm-desktop\binaries\crm-server-x86_64-pc-windows-msvc.exe -Force

Step '4/4 Программа и установщик'
Push-Location crates\crm-desktop
Run { npx -y '@tauri-apps/cli@2' build }
Pop-Location

$setup = Get-ChildItem target\release\bundle\nsis\*-setup.exe | Sort-Object LastWriteTime | Select-Object -Last 1
Write-Host "`nГотово: $($setup.FullName)" -ForegroundColor Green
Write-Host 'Установка поверх старой версии сохраняет данные.'
explorer.exe /select,"$($setup.FullName)"
