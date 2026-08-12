//go:build windows

package main

import (
	"io/fs"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/getlantern/systray"
)

// runTray 启动 Windows 系统托盘并阻塞, 直到用户退出或外部触发退出
func runTray(url string) {
	// 外部触发退出 (如 -stop 的 /shutdown 请求) 时结束托盘
	go func() {
		<-exitCh
		systray.Quit()
	}()

	// console 版 Ctrl+C 也走统一退出
	go func() {
		ch := make(chan os.Signal, 1)
		signal.Notify(ch, os.Interrupt, syscall.SIGTERM)
		<-ch
		shutdown()
		systray.Quit()
	}()

	systray.Run(func() {
		if icon, err := fs.ReadFile(webFS, "build/web_offline/favicon.ico"); err == nil {
			systray.SetIcon(icon)
		} else {
			log.Printf("读取托盘图标失败: %v", err)
		}
		systray.SetTitle("FluffyChat Portable")
		systray.SetTooltip("FluffyChat Portable")

		mOpen := systray.AddMenuItem("打开 FluffyChat", "在默认浏览器中打开")
		mQuit := systray.AddMenuItem("退出", "退出 FluffyChat Portable")

		go func() {
			for {
				select {
				case <-mOpen.ClickedCh:
					if err := openBrowser(url); err != nil {
						log.Printf("打开浏览器失败: %v", err)
					}
				case <-mQuit.ClickedCh:
					shutdown()
					systray.Quit()
					return
				}
			}
		}()
	}, nil)
}
