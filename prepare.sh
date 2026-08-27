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
import re, sys

def must(cond, msg):
    # 上游 index.html 结构变化时快速失败, 避免静默产出 canvaskit 走 CDN 的半离线包
    if not cond:
        print(f'[prepare.sh] FATAL: {msg} —— 上游 index.html 结构变化, 需人工更新本脚本',
              file=sys.stderr)
        sys.exit(1)

p = 'index.html'
s = open(p).read()

# 1. <base href> 改根路径 (否则本地协议下所有相对路径错乱)
must('<base href="/web/">' in s, '未找到 <base href="/web/">')
s = s.replace('<base href="/web/">', '<base href="/">')

# 2. 移除 Flutter 自带 SW 调用块 (缺失时无操作, 不强校验)
s = re.sub(r'serviceWorker: \{[^}]*\},', '', s)

# 3. canvaskit 本地化双保险: buildConfig.useLocalCanvasKit + load() config.canvasKitBaseUrl
#    任一失效 → canvaskit 回退从 gstatic CDN 拉取, 离线/国内加载又慢又可能失败
must('_flutter.buildConfig = {' in s, '未找到 _flutter.buildConfig 注入点')
s = s.replace('_flutter.buildConfig = {', '_flutter.buildConfig = {\n  useLocalCanvasKit: true,')
must('_flutter.loader.load({' in s, '未找到 _flutter.loader.load 注入点')
s = s.replace('_flutter.loader.load({', "_flutter.loader.load({\n        config: { canvasKitBaseUrl: 'canvaskit' },")

open(p, 'w').write(s)
print('index.html 改写完成 (base href / SW 移除 / canvaskit 本地化)')
PY

echo "完成: $(du -sh . | cut -f1) 文件数 $(find . -type f | wc -l)"
echo "可选: 放入自定义 config.json 预设默认 homeserver (参考上游 config.sample.json)"
