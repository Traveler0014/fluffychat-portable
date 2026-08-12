#!/usr/bin/env bash
# 构建脚本: Linux + Windows (x64/x86, 含 GUI 静默版)
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p dist
LDFLAGS="-s -w"   # 去符号, 减小体积
VERSION=$(grep -o '"version":"[^"]*"' build/web_offline/version.json 2>/dev/null | cut -d'"' -f4 || echo "unknown")

echo "== Linux x64 =="
CGO_ENABLED=0 go build -trimpath -ldflags "$LDFLAGS" -o dist/fluffychat-portable-linux-x64 .

echo "== Windows x64 (console) =="
CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -trimpath -ldflags "$LDFLAGS" -o "dist/fluffychat-portable-win64-v${VERSION}.exe" .

echo "== Windows x64 (GUI 静默版, 双击无黑窗) =="
CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -trimpath -ldflags "$LDFLAGS -H=windowsgui" -o "dist/fluffychat-portable-win64-gui-v${VERSION}.exe" .

echo "== Windows x86 / win32 (console) =="
CGO_ENABLED=0 GOOS=windows GOARCH=386 go build -trimpath -ldflags "$LDFLAGS" -o "dist/fluffychat-portable-win32-v${VERSION}.exe" .

ls -lh dist/
