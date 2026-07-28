#!/bin/sh
# Instalador de rtk-index.
#   curl -fsSL https://raw.githubusercontent.com/jesusamador-dev/rtk-mcp-server/main/install.sh | sh
#
# Descarga un binario precompilado si existe para tu plataforma; si no, cae a
# compilar con cargo. Coloca el comando `rtk-index` en tu PATH.
set -e

REPO="jesusamador-dev/rtk-mcp-server"
BIN="rtk-index"
INSTALL_DIR="${RTK_INSTALL_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
asset=""
case "$os/$arch" in
  Darwin/arm64) asset="rtk-index-macos-arm64" ;;
esac

echo "rtk-index installer  ($os/$arch)"

install_prebuilt() {
  url="https://github.com/$REPO/releases/latest/download/$1"
  mkdir -p "$INSTALL_DIR"
  echo "→ Descargando binario precompilado: $1"
  # A temporal + mv: reemplazar en un paso evita truncar un binario en uso
  # (`rtk-index update` se actualiza a sí mismo).
  tmp="$INSTALL_DIR/.$BIN.new.$$"
  if curl -fsSL "$url" -o "$tmp"; then
    chmod +x "$tmp"
    mv -f "$tmp" "$INSTALL_DIR/$BIN"
    echo "✓ Instalado en $INSTALL_DIR/$BIN"
    return 0
  fi
  rm -f "$tmp"
  echo "  (no disponible; intentaré compilar con cargo)"
  return 1
}

install_cargo() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "✗ No hay binario precompilado para $os/$arch y cargo no está instalado."
    echo "  Instala Rust desde https://rustup.rs y reintenta."
    exit 1
  fi
  echo "→ Compilando con cargo (puede tardar ~1-2 min)…"
  cargo install --git "https://github.com/$REPO" --bin "$BIN" --force
  INSTALL_DIR="$(dirname "$(command -v "$BIN")")"
  echo "✓ Instalado en $INSTALL_DIR/$BIN"
}

if [ -n "$asset" ]; then
  install_prebuilt "$asset" || install_cargo
else
  install_cargo
fi

# ¿Está el directorio de instalación en el PATH?
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "⚠ Añade $INSTALL_DIR a tu PATH (agrégalo a ~/.zshrc o ~/.bashrc):"
    echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

echo ""
echo "Siguiente paso — en la raíz de tu proyecto:"
echo "    rtk-index init ."
echo "Luego reinicia Claude Code. Verifica el entorno cuando quieras con: rtk-index check"
echo "Para actualizar más adelante:  rtk-index update"
