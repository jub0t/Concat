#!/usr/bin/env bash
# setup-android-build.sh — Preparar máquina de desarrollo para compilar Concat APK
# Ejecutar en Linux/macOS/Windows (WSL2) como usuario normal.
# NO ejecutar en Termux — no tiene NDK/SDK.

set -euo pipefail

echo "=== Concat Android Build Setup ==="
echo "Este script instala dependencias y configura el entorno para compilar el APK."
echo ""

# 1. Detectar OS
OS="$(uname -s)"
case "$OS" in
  Linux)   PKG_MGR="apt-get" ;;
  Darwin)  PKG_MGR="brew" ;;
  *)       echo "OS no soportado: $OS"; exit 1 ;;
esac

# 2. Instalar dependencias del sistema
echo "--- Instalando dependencias del sistema ---"
if [ "$PKG_MGR" = "apt-get" ]; then
  sudo apt-get update
  sudo apt-get install -y cmake ninja-build pkg-config libssl-dev openjdk-21-jdk-headless unzip curl git
elif [ "$PKG_MGR" = "brew" ]; then
  brew install cmake ninja pkg-config openssl openjdk@21 unzip curl git
fi

# 3. Configurar JAVA_HOME
if [ "$OS" = "Linux" ]; then
  export JAVA_HOME="/usr/lib/jvm/java-21-openjdk-amd64"
elif [ "$OS" = "Darwin" ]; then
  export JAVA_HOME="$(/usr/libexec/java_home -v 21)"
fi
echo "export JAVA_HOME=$JAVA_HOME" >> ~/.bashrc
echo "export PATH=\$JAVA_HOME/bin:\$PATH" >> ~/.bashrc

# 4. Instalar Android SDK cmdline-tools
echo "--- Instalando Android SDK cmdline-tools ---"
SDK_DIR="$HOME/android-sdk"
mkdir -p "$SDK_DIR"
cd "$SDK_DIR"
if [ ! -d "cmdline-tools" ]; then
  CMDLINE_URL="https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip"
  curl -L -o cmdline-tools.zip "$CMDLINE_URL"
  unzip -q cmdline-tools.zip
  mv cmdline-tools cmdline-tools-temp
  mkdir -p cmdline-tools/latest
  mv cmdline-tools-temp/* cmdline-tools/latest/
  rm -rf cmdline-tools-temp cmdline-tools.zip
fi
export ANDROID_SDK_ROOT="$SDK_DIR"
export PATH="$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:$PATH"
echo "export ANDROID_SDK_ROOT=$ANDROID_SDK_ROOT" >> ~/.bashrc
echo "export PATH=\$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:\$PATH" >> ~/.bashrc

# 5. Aceptar licencias e instalar platform-tools, build-tools, platforms
yes | sdkmanager --licenses
sdkmanager "platform-tools" "build-tools;34.0.0" "platforms;android-34" "ndk;27.0.12077973"

# 6. Configurar NDK
export ANDROID_NDK_HOME="$ANDROID_SDK_ROOT/ndk/27.0.12077973"
echo "export ANDROID_NDK_HOME=$ANDROID_NDK_HOME" >> ~/.bashrc

# 7. Targets Rust para Android
echo "--- Añadiendo targets Rust para Android ---"
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android

# 8. Configurar variables de entorno para Concat
REPO_DIR="$HOME/Concat"
if [ ! -d "$REPO_DIR" ]; then
  echo "Clonando repositorio Concat..."
  git clone https://github.com/username/Concat.git "$REPO_DIR"  # CAMBIA username/repo
fi
cd "$REPO_DIR/src"

# 9. Compilar FFmpeg para Android (usa script del repo)
echo "--- Compilando FFmpeg para Android ---"
bash ../scripts/mobile-env.sh
bash ../scripts/ffmpeg-mobile.sh
export FFMPEG_DIR="$(pwd)/ffmpeg-android"  # ajusta según salida del script

# 10. Compilar sherpa-onnx para Android
echo "--- Compilando sherpa-onnx para Android ---"
bash ../scripts/sherpa-mobile.sh
export SHERPA_ONNX_LIB_DIR="$(pwd)/sherpa-onnx-android"  # ajusta según salida del script

# 11. Exportar variables permanentes
cat >> ~/.bashrc <<EOF

# Concat Android build
export FFMPEG_DIR=$FFMPEG_DIR
export SHERPA_ONNX_LIB_DIR=$SHERPA_ONNX_LIB_DIR
export ANDROID_NDK_HOME=$ANDROID_NDK_HOME
export ANDROID_SDK_ROOT=$ANDROID_SDK_ROOT
export JAVA_HOME=$JAVA_HOME
export PATH=\$JAVA_HOME/bin:\$ANDROID_SDK_ROOT/cmdline-tools/latest/bin:\$PATH
EOF

# 12. Compilar concat-android
echo "--- Compilando concat-android ---"
cargo build --release -p concat-android --target aarch64-linux-android

echo ""
echo "=== ¡Setup completo! ==="
echo "Para compilar de nuevo en el futuro:"
echo "  source ~/.bashrc"
echo "  cd $REPO_DIR/src"
echo "  cargo build --release -p concat-android --target aarch64-linux-android"
echo ""
echo "El .so resultante está en:"
echo "  target/aarch64-linux-android/release/libconcat_android.so"
echo ""
echo "Para ensamblar el APK final, usa Gradle en concat-android/ (requiere Android Studio o ./gradlew)."