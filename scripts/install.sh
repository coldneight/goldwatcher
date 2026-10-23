#!/usr/bin/env bash
set -euo pipefail

# 金瞳 GoldWatcher — Linux per-user installer.
# Builds a release binary, installs it under ~/.local, and registers a desktop
# entry with the application icon.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN_DIR="$PREFIX/bin"
APP_DIR="$PREFIX/share/applications"
ICON_ROOT="$PREFIX/share/icons/hicolor"

cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
  echo "未找到 cargo，请先安装 Rust：https://rustup.rs" >&2
  exit 1
fi

echo "==> 构建 release 版本"
cargo build --release --target-dir "$TARGET_DIR"

install -d "$BIN_DIR" "$APP_DIR" \
  "$ICON_ROOT/16x16/apps" "$ICON_ROOT/32x32/apps" "$ICON_ROOT/48x48/apps" \
  "$ICON_ROOT/64x64/apps" "$ICON_ROOT/128x128/apps" "$ICON_ROOT/256x256/apps" \
  "$ICON_ROOT/512x512/apps"

install -m 755 "$TARGET_DIR/release/gold-watcher" "$BIN_DIR/gold-watcher"
for size in 16 32 48 64 128 256 512; do
  src="assets/icon-$size.png"
  if [[ -f "$src" ]]; then
    install -m 644 "$src" "$ICON_ROOT/${size}x${size}/apps/gold-watcher.png"
  fi
done
install -m 644 assets/icon-256.png "$ICON_ROOT/256x256/apps/gold-watcher.png"

cat > "$APP_DIR/gold-watcher.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.0
Name=金瞳 GoldWatcher
Name[en]=GoldWatcher
Comment=全球黄金市场实时监控与开收盘/价格告警
Comment[en]=Global gold market monitor with price and session alerts
Exec="$BIN_DIR/gold-watcher"
Icon=gold-watcher
Terminal=false
Categories=Office;Finance;
Keywords=gold;XAU;finance;alert;黄金;行情;
StartupNotify=true
StartupWMClass=io.github.goldwatcher
DESKTOP

chmod 644 "$APP_DIR/gold-watcher.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICON_ROOT" >/dev/null 2>&1 || true
fi

echo "已安装：$BIN_DIR/gold-watcher"
echo "桌面入口：$APP_DIR/gold-watcher.desktop"
echo "如果桌面环境没有立即显示图标，请注销后重新登录。"
