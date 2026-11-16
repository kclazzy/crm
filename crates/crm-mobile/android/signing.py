"""Добавляет в build.gradle.kts подпись выпуска ключом из переменных окружения."""
import sys

path = sys.argv[1]
text = open(path, encoding="utf-8").read()
if "signingConfigs" in text:
    sys.exit(0)

signing = '''    signingConfigs {
        create("release") {
            storeFile = file("release.p12")
            storeType = "pkcs12"
            storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("ANDROID_KEY_ALIAS") ?: "crm"
            keyPassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
        }
    }
    buildTypes {'''
assert "    buildTypes {" in text, "не найден блок buildTypes"
text = text.replace("    buildTypes {", signing, 1)
marker = 'getByName("release") {'
assert marker in text, "не найден тип сборки release"
text = text.replace(marker, marker + '\n            signingConfig = signingConfigs.getByName("release")', 1)
open(path, "w", encoding="utf-8").write(text)
