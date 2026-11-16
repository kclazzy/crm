#!/usr/bin/env bash
# Доработка проекта Android после `tauri android init` (папка gen/android создаётся
# заново при каждой сборке, поэтому правки живут здесь, а не в ней).
set -euo pipefail
cd "$(dirname "$0")/.."
app=gen/android/app

# Окно приложения открывает CRM через собственный туннель http://127.0.0.1:ПОРТ.
# Незашифрованный HTTP разрешаем только для этого адреса: наружу трафик уходит
# уже зашифрованным (TLS до сервера или QUIC по ключу сервера).
mkdir -p "$app/src/main/res/xml"
cp android/network_security_config.xml "$app/src/main/res/xml/network_security_config.xml"
manifest="$app/src/main/AndroidManifest.xml"
if ! grep -q networkSecurityConfig "$manifest"; then
  sed -i '0,/<application/s//<application android:networkSecurityConfig="@xml\/network_security_config"/' "$manifest"
fi
grep -q 'networkSecurityConfig="@xml/network_security_config"' "$manifest"

# Подпись выпуска: ключ приходит из секретов сборки (см. docs/ANDROID.md).
if [ -n "${ANDROID_KEYSTORE_BASE64:-}" ]; then
  echo "$ANDROID_KEYSTORE_BASE64" | base64 -d > "$app/release.p12"
  python3 android/signing.py "$app/build.gradle.kts"
fi
