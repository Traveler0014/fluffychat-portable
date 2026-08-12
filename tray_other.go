//go:build !windows

package main

import (
	"fmt"
	"os"
	"os/signal"
	"syscall"
)

// runTray 非 Windows 平台: 无托盘, 阻塞等待 Ctrl+C / SIGTERM 或外部触发退出
func runTray(url string) {
	ch := make(chan os.Signal, 1)
	signal.Notify(ch, os.Interrupt, syscall.SIGTERM)
	select {
	case <-ch:
		fmt.Println("\n收到退出信号")
		shutdown()
	case <-exitCh:
	}
}
