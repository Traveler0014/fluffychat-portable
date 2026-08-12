// FluffyChat Portable —— 把 FluffyChat Web 打包为离线单二进制
//
// 用法:
//   fluffychat-portable                     # 默认端口 8800, 自动打开浏览器
//   fluffychat-portable -port 0             # 随机端口
//   fluffychat-portable -homeserver xxx     # 预设默认 Matrix 服务器 (写入运行时 config.json)
//   fluffychat-portable -dir ./web          # 调试: 从外部目录加载资源 (免重新编译)
//   fluffychat-portable -no-open            # 不自动打开浏览器
//   fluffychat-portable -stop               # 请求运行中的实例退出
//
// 退出方式:
//   Windows: 系统托盘图标 → 右键 → 退出 (托盘常驻)
//   Linux/其他: Ctrl+C
//   任意平台: -stop 请求运行中的实例退出
package main

import (
	"embed"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"sync"
	"time"
)

//go:embed all:build/web_offline
var webFS embed.FS

const (
	healthToken    = "fluffychat-portable"
	defaultPort    = 8800
	defaultAppName = "FluffyChat"
)

// exitCh 关闭时表示程序应退出; shutdown() 全局触发一次
var (
	exitCh       = make(chan struct{})
	shutdownOnce sync.Once
)

func shutdown() { shutdownOnce.Do(func() { close(exitCh) }) }

func main() {
	port := flag.Int("port", defaultPort, "监听端口 (0 = 随机空闲端口)")
	noOpen := flag.Bool("no-open", false, "不自动打开浏览器")
	homeserver := flag.String("homeserver", "", "预设默认 Matrix homeserver (写入运行时 config.json)")
	preset := flag.String("preset", "", "登录页可选的预设服务器列表 (逗号分隔, 优先级高于 -homeserver)")
	dir := flag.String("dir", "", "调试: 从外部目录提供 web 资源, 而不是内置的")
	stop := flag.Bool("stop", false, "请求运行中的实例退出")
	flag.Parse()

	// -stop: 只探测并请求已有实例退出, 不启动服务
	if *stop {
		os.Exit(requestStop(*port))
	}

	// 组装运行时 config.json 内容
	cfg := renderConfig(*homeserver, *preset)

	// 选择 web 资源来源
	var web fs.FS
	if *dir != "" {
		web = os.DirFS(*dir)
		log.Printf("使用外部资源目录: %s", *dir)
	} else {
		sub, err := fs.Sub(webFS, "build/web_offline")
		if err != nil {
			log.Fatalf("读取内置资源失败: %v", err)
		}
		web = sub
	}

	// 监听端口 (含单实例探测)
	addr, ln, alreadyRunning, err := listen(*port)
	if err != nil {
		log.Fatalf("监听失败: %v", err)
	}
	if alreadyRunning {
		// 已有实例在跑: 直接唤醒浏览器后退出
		url := "http://" + addr + "/"
		log.Printf("检测到已有实例 (%s), 唤醒浏览器后退出", url)
		_ = openBrowser(url)
		return
	}

	// 组装 HTTP 路由
	handler := buildHandler(web, cfg)

	srv := &http.Server{Handler: handler}
	go func() {
		if err := srv.Serve(ln); err != nil && err != http.ErrServerClosed {
			log.Fatalf("server error: %v", err)
		}
	}()
	go func() {
		<-exitCh
		fmt.Println("\n已退出")
		_ = srv.Close()
	}()

	url := "http://" + addr + "/"
	fmt.Printf("\n  FluffyChat Portable 已启动\n  URL: %s\n", url)
	if runtime.GOOS == "windows" {
		fmt.Println("  已驻留系统托盘 (图标 → 右键 → 退出)")
	} else {
		fmt.Println("  按 Ctrl+C 退出")
	}
	fmt.Printf("  也可运行: %s -stop\n\n", os.Args[0])

	if !*noOpen {
		if err := openBrowser(url); err != nil {
			log.Printf("自动打开浏览器失败: %v (可手动访问 %s)", err, url)
		}
	}

	// 平台层阻塞: Windows = 托盘; 其他 = 等待信号
	runTray(url)

	// runTray 返回后等待真正关闭
	<-exitCh
	_ = srv.Close()
}

// listen 在 127.0.0.1 上监听。port=0 时选随机空闲端口;
// port>0 且被占用时, 若占用者是自己实例则返回 alreadyRunning=true, 否则回退随机端口。
func listen(port int) (addr string, ln net.Listener, alreadyRunning bool, err error) {
	if port == 0 {
		l, e := net.Listen("tcp", "127.0.0.1:0")
		if e != nil {
			return "", nil, false, e
		}
		return l.Addr().String(), l, false, nil
	}
	fixed := fmt.Sprintf("127.0.0.1:%d", port)
	if isSelf(fixed) {
		return fixed, nil, true, nil
	}
	l, e := net.Listen("tcp", fixed)
	if e == nil {
		return l.Addr().String(), l, false, nil
	}
	// 端口被其它程序占用 → 随机端口
	log.Printf("端口 %d 被占用, 改用随机端口", port)
	l, e = net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		return "", nil, false, e
	}
	return l.Addr().String(), l, false, nil
}

// isSelf 探测 addr 上是否运行着本程序实例 (通过 /health 握手)
func isSelf(addr string) bool {
	client := &http.Client{Timeout: 500 * time.Millisecond}
	resp, err := client.Get("http://" + addr + "/health")
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(io.LimitReader(resp.Body, 64))
	return strings.TrimSpace(string(b)) == healthToken
}

// buildHandler 组装 HTTP 路由: 静态资源 + 动态 config.json + 健康检查 + 关闭接口
func buildHandler(web fs.FS, cfg []byte) http.Handler {
	fileServer := http.FileServer(http.FS(web))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-cache")
		switch r.URL.Path {
		case "/health":
			_, _ = w.Write([]byte(healthToken))
			return
		case "/config.json":
			w.Header().Set("Content-Type", "application/json; charset=utf-8")
			_, _ = w.Write(cfg)
			return
		case "/shutdown":
			// 仅本机可访问 (127.0.0.1 监听), 供 -stop 使用
			shutdown()
			_, _ = w.Write([]byte("bye"))
			return
		}
		fileServer.ServeHTTP(w, r)
	})
}

// requestStop 探测 -port 上的实例并请求其退出。返回进程退出码。
func requestStop(port int) int {
	if port == 0 {
		log.Println("-stop 需要指定与实例一致的端口, 如: -stop -port 8800")
		return 2
	}
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	if !isSelf(addr) {
		log.Printf("%s 上没有运行中的实例", addr)
		return 1
	}
	client := &http.Client{Timeout: 3 * time.Second}
	resp, err := client.Post("http://"+addr+"/shutdown", "", nil)
	if err != nil {
		log.Printf("请求退出失败: %v", err)
		return 1
	}
	defer resp.Body.Close()
	_, _ = io.Copy(io.Discard, resp.Body)
	log.Printf("已请求 %s 退出", addr)
	return 0
}

// renderConfig 生成 FluffyChat 运行时配置 (对应上游 config.sample.json 字段)。
// homeserver / preset 为空时不覆盖上游默认值。
func renderConfig(homeserver, preset string) []byte {
	cfg := map[string]any{
		"applicationName":            defaultAppName,
		"defaultHomeserver":          "matrix.org",
		"presetHomeserver":           "",
		"welcomeText":                "",
		"privacyUrl":                 "https://github.com/krille-chan/fluffychat/blob/main/PRIVACY.md",
		"audioRecordingNumChannels":  1,
		"audioRecordingAutoGain":     true,
		"audioRecordingEchoCancel":   false,
		"audioRecordingNoiseSuppress": true,
		"audioRecordingBitRate":      64000,
		"audioRecordingSamplingRate": 44100,
		"renderHtml":                 true,
		"fontSizeFactor":             1,
		"messagePreviewMaxLines":     128,
		"hideRedactedEvents":         false,
		"hideUnknownEvents":          true,
		"separateChatTypes":          false,
		"autoplayImages":             true,
		"sendTypingNotifications":    true,
		"sendPublicReadReceipts":     true,
		"swipeRightToLeftToReply":    true,
		"sendOnEnter":                false,
		"showPresences":              true,
		"displayNavigationRail":      false,
		"experimentalVoip":           false,
		"shareKeysWith":              "all",
		"noEncryptionWarningShown":   false,
		"displayChatDetailsColumn":   false,
		"colorSchemeSeedInt":         int64(4283835834),
		"enableSoftLogout":           false,
	}
	if homeserver != "" {
		cfg["defaultHomeserver"] = homeserver
	}
	if preset != "" {
		cfg["presetHomeserver"] = strings.Join(strings.FieldsFunc(preset, func(r rune) bool { return r == ',' || r == ';' }), ",")
	} else if homeserver != "" {
		cfg["presetHomeserver"] = homeserver
	}
	b, _ := json.MarshalIndent(cfg, "", "  ")
	return b
}

// openBrowser 打开系统默认浏览器
func openBrowser(url string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "windows":
		cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
	case "darwin":
		cmd = exec.Command("open", url)
	default:
		cmd = exec.Command("xdg-open", url)
	}
	return cmd.Start()
}
