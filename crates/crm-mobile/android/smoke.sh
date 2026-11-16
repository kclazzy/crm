#!/usr/bin/env bash
# Проверка на эмуляторе: приложение ставится, запускается и показывает экран серверов.
set -uo pipefail
out=out/smoke
mkdir -p "$out"
adb install -r out/CRM.apk || exit 1
adb logcat -c
adb shell monkey -p com.kclazzy.crm -c android.intent.category.LAUNCHER 1 >/dev/null
ok=1
for i in $(seq 1 20); do
  sleep 3
  adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1
  adb pull /sdcard/ui.xml "$out/ui.xml" >/dev/null 2>&1
  if grep -q 'Добавить сервер' "$out/ui.xml" 2>/dev/null; then ok=0; break; fi
done
adb exec-out screencap -p > "$out/screen.png"
adb logcat -d > "$out/logcat.txt"
if [ $ok -ne 0 ]; then
  echo "Экран серверов не появился. Последние ошибки:"
  grep -E 'FATAL|AndroidRuntime|com.kclazzy.crm|RustStdoutStderr|chromium' "$out/logcat.txt" | tail -40
  exit 1
fi
echo "Экран серверов показан."
# Открыть окно добавления сервера — проверка, что сценарии страницы работают.
adb shell input tap $(python3 - "$out/ui.xml" <<'PY'
import re, sys
xml = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r'text="Добавить сервер"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)
x1, y1, x2, y2 = map(int, m.groups())
print((x1 + x2) // 2, (y1 + y2) // 2)
PY
)
sleep 3
adb shell uiautomator dump /sdcard/ui2.xml >/dev/null 2>&1
adb pull /sdcard/ui2.xml "$out/ui2.xml" >/dev/null 2>&1
adb exec-out screencap -p > "$out/screen-add.png"
grep -q 'Сканировать QR-код' "$out/ui2.xml" || { echo "Окно «Добавить сервер» не открылось"; exit 1; }
echo "Окно «Добавить сервер» открыто."

# Настоящее подключение: ключ сервера CRM, запущенного рядом, → страница входа CRM
# в окне приложения через локальный туннель (http://127.0.0.1).
[ -s out/key.txt ] || { echo "Ключа сервера нет — подключение не проверяется."; exit 0; }
adb shell input tap $(python3 - "$out/ui2.xml" <<'PY'
import re, sys
xml = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r'class="android\.widget\.EditText"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)
x1, y1, x2, y2 = map(int, m.groups())
print((x1 + x2) // 2, (y1 + y2) // 2)
PY
)
sleep 1
adb shell input text "$(cat out/key.txt)"
sleep 2
adb exec-out screencap -p > "$out/screen-key.png"
# К кнопке «Добавить» — клавишами: экранная клавиатура может её закрывать.
adb shell input keyevent 61; adb shell input keyevent 61; adb shell input keyevent 66
ok=1
for i in $(seq 1 30); do
  sleep 3
  adb shell uiautomator dump /sdcard/ui3.xml >/dev/null 2>&1
  adb pull /sdcard/ui3.xml "$out/ui3.xml" >/dev/null 2>&1
  if grep -q 'Войти' "$out/ui3.xml" 2>/dev/null; then ok=0; break; fi
done
adb exec-out screencap -p > "$out/screen-crm.png"
adb logcat -d > "$out/logcat.txt"
if [ $ok -ne 0 ]; then
  echo "Страница входа CRM не открылась. Что на экране:"
  grep -o 'text="[^"]\{3,\}"' "$out/ui3.xml" | head -30
  grep -E 'RustStdoutStderr|chromium|net::' "$out/logcat.txt" | tail -20
  exit 1
fi
echo "Страница входа CRM открыта через приложение."
