$ErrorActionPreference = "Stop"

$ROOT_DIR = Split-Path -Parent $PSScriptRoot
$ANDROID_DIR = Join-Path $ROOT_DIR "android_client"
$BIN_DIR = Join-Path $ROOT_DIR "bin"

if (-not $env:JAVA_HOME) {
    $env:JAVA_HOME = "C:\Program Files\Microsoft\jdk-25.0.5.21-hotspot"
}
$SDK_DIR = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { "C:\Users\T\AppData\Local\Android\Sdk" }
$NDK_DIR = Join-Path $SDK_DIR "ndk\30.0.16248370"
$env:ANDROID_NDK_HOME = $NDK_DIR

$BUILD_TOOLS = Join-Path $SDK_DIR "build-tools\36.1.0"
$ANDROID_JAR = Join-Path $SDK_DIR "platforms\android-34\android.jar"

$JBR_BIN = "C:\Program Files\Android\Android Studio\jbr\bin"
if (Test-Path (Join-Path $JBR_BIN "javac.exe")) {
    $JAVA_BIN = $JBR_BIN
} else {
    $JAVA_BIN = Join-Path $env:JAVA_HOME "bin"
}
$JAVAC = Join-Path $JAVA_BIN "javac.exe"
$JAR = Join-Path $JAVA_BIN "jar.exe"
$KEYTOOL = Join-Path $JAVA_BIN "keytool.exe"
$D8 = Join-Path $BUILD_TOOLS "d8.bat"
$AAPT2 = Join-Path $BUILD_TOOLS "aapt2.exe"
$ZIPALIGN = Join-Path $BUILD_TOOLS "zipalign.exe"
$APKSIGNER = Join-Path $BUILD_TOOLS "apksigner.bat"

Write-Host ">>> 1. Compiling Rust Android JNI Core (libiconnect_core.so) with cargo-ndk..."
Push-Location $ROOT_DIR
try {
    cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -P 21 -o "$ANDROID_DIR\libs" build -p i_connect_android_core --release --offline
    if ($LASTEXITCODE -ne 0) { throw "cargo-ndk build failed" }
} finally {
    Pop-Location
}

Write-Host ">>> 2. Preparing Android APK build directories..."
$BUILD_DIR = Join-Path $ANDROID_DIR "build"
if (Test-Path $BUILD_DIR) { Remove-Item -Recurse -Force $BUILD_DIR }
New-Item -ItemType Directory -Force -Path `
    "$BUILD_DIR\gen", `
    "$BUILD_DIR\obj", `
    "$BUILD_DIR\dex", `
    "$BUILD_DIR\apk_temp\lib\arm64-v8a", `
    "$BUILD_DIR\apk_temp\lib\armeabi-v7a", `
    "$BUILD_DIR\apk_temp\lib\x86_64" | Out-Null

Write-Host ">>> 3. Compiling Android resources with aapt2..."
& $AAPT2 compile --dir "$ANDROID_DIR\res" -o "$BUILD_DIR\compiled_res.zip"
if ($LASTEXITCODE -ne 0) { throw "aapt2 compile failed" }

Write-Host ">>> 4. Linking AndroidManifest & resources with aapt2..."
& $AAPT2 link -I $ANDROID_JAR --manifest "$ANDROID_DIR\AndroidManifest.xml" --min-sdk-version 21 --target-sdk-version 34 -o "$BUILD_DIR\res.apk" --java "$BUILD_DIR\gen" "$BUILD_DIR\compiled_res.zip" --auto-add-overlay
if ($LASTEXITCODE -ne 0) { throw "aapt2 link failed" }

Write-Host ">>> 5. Compiling Java Android shell sources with javac..."
$JAVA_FILES = Get-ChildItem -Path "$BUILD_DIR\gen", "$ANDROID_DIR\src" -Recurse -Filter "*.java" | Select-Object -ExpandProperty FullName
& $JAVAC -source 8 -target 8 -encoding UTF-8 -cp $ANDROID_JAR -d "$BUILD_DIR\obj" $JAVA_FILES
if ($LASTEXITCODE -ne 0) { throw "javac failed" }

Write-Host ">>> 6. Dexing classes with d8..."
& $JAR cf "$BUILD_DIR\app.jar" -C "$BUILD_DIR\obj" .
& cmd.exe /c "`"$D8`" --lib `"$ANDROID_JAR`" --output `"$BUILD_DIR\dex`" `"$BUILD_DIR\app.jar`""
if ($LASTEXITCODE -ne 0) { throw "d8 dexing failed" }

Write-Host ">>> 7. Packaging classes.dex & Rust .so libraries into APK..."
Copy-Item "$BUILD_DIR\res.apk" "$BUILD_DIR\unaligned.apk"
Copy-Item "$ANDROID_DIR\libs\arm64-v8a\libiconnect_core.so" "$BUILD_DIR\apk_temp\lib\arm64-v8a\"
Copy-Item "$ANDROID_DIR\libs\armeabi-v7a\libiconnect_core.so" "$BUILD_DIR\apk_temp\lib\armeabi-v7a\"
Copy-Item "$ANDROID_DIR\libs\x86_64\libiconnect_core.so" "$BUILD_DIR\apk_temp\lib\x86_64\"
Copy-Item "$BUILD_DIR\dex\classes.dex" "$BUILD_DIR\apk_temp\"

python -c "
import zipfile, os, sys
apk, temp_dir = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(apk, 'a', compression=zipfile.ZIP_DEFLATED) as z:
    for root, _, files in os.walk(temp_dir):
        for f in files:
            full = os.path.join(root, f)
            arc = os.path.relpath(full, temp_dir).replace(os.sep, '/')
            z.write(full, arc)
" "$BUILD_DIR\unaligned.apk" "$BUILD_DIR\apk_temp"
if ($LASTEXITCODE -ne 0) { throw "APK zip injection failed" }

Write-Host ">>> 8. Aligning APK with zipalign..."
& $ZIPALIGN -p -f 4 "$BUILD_DIR\unaligned.apk" "$BUILD_DIR\aligned.apk"
if ($LASTEXITCODE -ne 0) { throw "zipalign failed" }

Write-Host ">>> 9. Signing I_Connect.apk..."
$KEYSTORE = Join-Path $ANDROID_DIR "debug.keystore"
if (-not (Test-Path $KEYSTORE)) {
    & $KEYTOOL -genkey -v -keystore $KEYSTORE -storepass android -alias androiddebugkey -keypass android -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=I Connect,O=IConnect,C=US"
}

New-Item -ItemType Directory -Force -Path $BIN_DIR | Out-Null
$FINAL_APK = Join-Path $BIN_DIR "I_Connect.apk"
& cmd.exe /c "`"$APKSIGNER`" sign --ks `"$KEYSTORE`" --ks-pass pass:android --ks-key-alias androiddebugkey --key-pass pass:android --out `"$FINAL_APK`" `"$BUILD_DIR\aligned.apk`""
if ($LASTEXITCODE -ne 0) { throw "apksigner failed" }

Write-Host ">>> [SUCCESS] Signed Android APK ready: $FINAL_APK"
Get-Item $FINAL_APK | Select-Object Name, Length, LastWriteTime
