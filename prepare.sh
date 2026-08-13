#!/usr/bin/env bash
# FluffyChat Web 离线包裁剪脚本
# 用法: ./prepare.sh [version]   (默认 v2.8.0)
# 产物: ./build/web_offline/ (可直接嵌入 Tauri/Go)
set -euo pipefail

VERSION="${1:-v2.8.0}"
TARBALL="fluffychat-web-${VERSION}.tar.gz"
BASE="https://github.com/krille-chan/fluffychat/releases/download/${VERSION}/fluffychat-web.tar.gz"

echo "[1/4] 下载 release web 包 (国内可用加速镜像, 失败时换 ghfast.top 前缀)..."
if [ ! -f "$TARBALL" ]; then
  curl -L -o "$TARBALL" "$BASE" || curl -L -o "$TARBALL" "https://gh.ddlc.top/$BASE"
fi

echo "[2/4] 解压..."
rm -rf build/web_offline && mkdir -p build/web_offline
tar xzf "$TARBALL" -C build/web_offline --strip-components=2 build/web

cd build/web_offline
echo "[3/4] 裁剪冗余文件..."
# 调试用 source map (≈8MB)
rm -f main.dart.js.map main.dart.js_*.part.js.map native_executor.js.map native_executor.js.deps
# 非目标渲染器 + 调试符号 (≈23MB): 仅保留 canvaskit(js/wasm) + chromium 变体
rm -rf canvaskit/skwasm* canvaskit/wimp* canvaskit/experimental_webparagraph
find . -name '*.symbols' -delete
# Flutter 自带 SW (本地服务不需要, 避免版本缓存)
rm -f flutter_service_worker.js .last_build_id

# 托盘图标: 从应用图标生成多尺寸 ICO (供系统托盘/窗口图标使用)
python3 - <<'PY' || true
from PIL import Image
img = Image.open('icons/Icon-192.png').convert('RGBA')
img.save('favicon.ico', sizes=[(16,16),(32,32),(48,48),(64,64)])
print('favicon.ico generated')
PY

# Tauri 图标: 生成 src-tauri/icons/ (窗口/托盘/打包图标)
mkdir -p ../../src-tauri/icons
python3 - <<'PY' || true
from PIL import Image
src = Image.open('icons/Icon-192.png').convert('RGBA')
src.resize((512, 512), Image.LANCZOS).save('../../src-tauri/icons/icon.png')
src.resize((32, 32), Image.LANCZOS).save('../../src-tauri/icons/32x32.png')
src.resize((128, 128), Image.LANCZOS).save('../../src-tauri/icons/128x128.png')
src.resize((256, 256), Image.LANCZOS).save('../../src-tauri/icons/128x128@2x.png')
src.save('../../src-tauri/icons/icon.ico', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
print('tauri icons generated')
PY

echo "[4/4] 改写 index.html (base href / canvaskit 本地化 / 禁用 SW)..."
python3 - <<'PY'
import re
p = 'index.html'
s = open(p).read()
s = s.replace('<base href="/web/">', '<base href="/">')
# 移除 Flutter 自带 SW 调用块
s = re.sub(r'serviceWorker: \{[^}]*\},', '', s)
# 双保险: 该版本 loader 从 buildConfig 读 useLocalCanvasKit; load() config 同时给 canvasKitBaseUrl
s = s.replace('_flutter.buildConfig = {', '_flutter.buildConfig = {\n  useLocalCanvasKit: true,')
s = s.replace('_flutter.loader.load({', "_flutter.loader.load({\n        config: { canvasKitBaseUrl: 'canvaskit' },")
open(p, 'w').write(s)
PY

echo "完成: $(du -sh . | cut -f1) 文件数 $(find . -type f | wc -l)"
echo "可选: 放入自定义 config.json 预设默认 homeserver (参考上游 config.sample.json)"
