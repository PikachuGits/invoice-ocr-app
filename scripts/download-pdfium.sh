#!/usr/bin/env bash
# 下载 bblanchon/pdfium-binaries 的动态库到 src-tauri/resources/，供打包使用。
# 用法: ./scripts/download-pdfium.sh [版本号]   (默认 7961，与 pdfium-auto 保持一致)
set -euo pipefail

cd "$(dirname "$0")/.."

PDFIUM_VERSION="${1:-7961}"
BASE="https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F${PDFIUM_VERSION}"
MIRRORS=("https://gh-proxy.com/" "https://ghfast.top/" "")
DEST="src-tauri/resources"
mkdir -p "$DEST"

fetch() { # fetch <archive>
  local archive="$1"
  local url
  for m in "${MIRRORS[@]}"; do
    url="${m}${BASE}/${archive}"
    echo ">> 尝试: ${url}"
    if curl -fSL --retry 2 --max-time 600 -o "/tmp/${archive}" "$url" 2>/dev/null; then
      echo "   OK"
      return 0
    fi
  done
  echo "!! 下载失败: ${archive}（请检查网络或更换镜像）" >&2
  return 1
}

# 根据当前平台下载对应库（可自行改为需要的平台）
case "$(uname -s)" in
  Darwin)
    case "$(uname -m)" in
      arm64) fetch "pdfium-mac-arm64.tgz" && tar xzf "/tmp/pdfium-mac-arm64.tgz" -C /tmp lib/libpdfium.dylib && cp /tmp/lib/libpdfium.dylib "$DEST/libpdfium.dylib" ;;
      x86_64) fetch "pdfium-mac-x64.tgz" && tar xzf "/tmp/pdfium-mac-x64.tgz" -C /tmp lib/libpdfium.dylib && cp /tmp/lib/libpdfium.dylib "$DEST/libpdfium.dylib" ;;
    esac
    ;;
  Linux) fetch "pdfium-linux-x64.tgz" && tar xzf "/tmp/pdfium-linux-x64.tgz" -C /tmp lib/libpdfium.so && cp /tmp/lib/libpdfium.so "$DEST/libpdfium.so" ;;
  MINGW*|MSYS*|CYGWIN*)
    fetch "pdfium-win-x64.tgz" && tar xzf "/tmp/pdfium-win-x64.tgz" -C /tmp bin/pdfium.dll && cp /tmp/bin/pdfium.dll "$DEST/pdfium.dll" ;;
  *) echo "!! 不支持的平台" >&2; exit 1 ;;
esac

echo "== 完成: $DEST"
ls -la "$DEST"
