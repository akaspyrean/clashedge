package mihomo

import (
	"context"
	"fmt"
	"sync"

	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub"
	"github.com/metacubex/mihomo/log"
	"github.com/metacubex/mihomo/config"
)

var (
	core    *hub.Hub
	coreMu  sync.Mutex
	started bool
)

// ProxyInfo represents a proxy for UI
type ProxyInfo struct {
	Name   string `json:"name"`
	Type   string `json:"type"`
	UDP    bool   `json:"udp"`
	Alive  bool   `json:"alive"`
	Delay  int    `json:"delay"`
}

// Start initializes and starts mihomo core with YAML config
// configYAML: full mihomo config in YAML format
// tunFD: file descriptor from Android VpnService (0 if not using TUN)
// Returns empty string on success, error message on failure
func Start(configYAML string, tunFD int) string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if started {
		return "already started"
	}

	// Parse YAML directly using mihomo's built-in parser
	mihomoCfg, err := config.Parse([]byte(configYAML))
	if err != nil {
		return "parse config: " + err.Error()
	}

	// If tunFD provided, inject it via device path (/proc/self/fd/N)
	if tunFD > 0 {
		mihomoCfg.Tun.Device = fmt.Sprintf("/proc/self/fd/%d", tunFD)
	}

	// Initialize logging
	log.SetLevel(log.INFO)

	// Create hub
	h := hub.New()
	ctx := context.Background()

	if err := h.Parse(ctx, mihomoCfg); err != nil {
		return "parse: " + err.Error()
	}

	if err := h.Start(ctx); err != nil {
		return "start: " + err.Error()
	}

	core = h
	started = true
	return ""
}

// Stop stops the mihomo core
func Stop() string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started || core == nil {
		return "not running"
	}

	core.Stop()
	core = nil
	started = false
	return ""
}

// SetMode changes the proxy mode (Rule, Global, Direct)
func SetMode(mode string) string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started || core == nil {
		return "not running"
	}

	ctx := context.Background()
	if err := core.SetMode(ctx, C.Mode(mode)); err != nil {
		return err.Error()
	}
	return ""
}

// GetProxies returns list of all proxies
func GetProxies() []ProxyInfo {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started || core == nil {
		return nil
	}

	proxies := core.Proxies()
	result := make([]ProxyInfo, 0, len(proxies))
	for name, p := range proxies {
		result = append(result, ProxyInfo{
			Name: name,
			Type: p.Type().String(),
			UDP:  p.SupportUDP(),
		})
	}
	return result
}

// SelectProxy selects a proxy in a group
func SelectProxy(groupName, proxyName string) string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started || core == nil {
		return "not running"
	}

	ctx := context.Background()
	if err := core.SelectProxy(ctx, groupName, proxyName); err != nil {
		return err.Error()
	}
	return ""
}

// GetVersion returns mihomo version
func GetVersion() string {
	return C.Version
}

// IsRunning returns whether core is running
func IsRunning() bool {
	coreMu.Lock()
	defer coreMu.Unlock()
	return started && core != nil
}