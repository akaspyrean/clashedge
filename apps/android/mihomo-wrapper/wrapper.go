package mihomo

import (
	"context"
	"encoding/json"
	"errors"
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

// Config holds simplified configuration for gomobile
type Config struct {
	MixedPort     int      `json:"mixed-port"`
	ExternalUI    string   `json:"external-ui"`
	ExternalCtrl  string   `json:"external-controller"`
	Secret        string   `json:"secret"`
	Mode          string   `json:"mode"`
	LogLevel      string   `json:"log-level"`
	AllowLan      bool     `json:"allow-lan"`
	DNSServers    []string `json:"dns-servers"`
	Proxies       []ProxyConfig `json:"proxies"`
	ProxyGroups   []GroupConfig `json:"proxy-groups"`
	Rules         []string `json:"rules"`
}

type ProxyConfig struct {
	Name     string            `json:"name"`
	Type     string            `json:"type"`
	Server   string            `json:"server"`
	Port     int               `json:"port"`
	Password string            `json:"password"`
	Options  map[string]string `json:"options,omitempty"`
}

type GroupConfig struct {
	Name    string   `json:"name"`
	Type    string   `json:"type"`
	Proxies []string `json:"proxies"`
	Options map[string]string `json:"options,omitempty"`
}

// ProxyInfo represents a proxy for UI
type ProxyInfo struct {
	Name   string `json:"name"`
	Type   string `json:"type"`
	UDP    bool   `json:"udp"`
	Alive  bool   `json:"alive"`
	Delay  int    `json:"delay"`
}

// Start initializes and starts mihomo core with JSON config
// Returns empty string on success, error message on failure
func Start(configJSON string) string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if started {
		return "already started"
	}

	var cfg Config
	if err := json.Unmarshal([]byte(configJSON), &cfg); err != nil {
		return "parse config: " + err.Error()
	}

	// Convert to mihomo config
	mihomoCfg := convertConfig(cfg)

	// Initialize logging
	log.SetLevel(parseLogLevel(cfg.LogLevel))

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
	if err := core.SetMode(ctx, mode); err != nil {
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

func convertConfig(cfg Config) config.Config {
	// Minimal conversion - in production, implement full conversion
	// This is a placeholder that returns a basic config
	return config.Config{
		MixedPort:     cfg.MixedPort,
		ExternalUI:    cfg.ExternalUI,
		ExternalCtrl:  cfg.ExternalCtrl,
		Secret:        cfg.Secret,
		Mode:          cfg.Mode,
		LogLevel:      cfg.LogLevel,
		AllowLan:      cfg.AllowLan,
		// Proxies, Groups, Rules would need full conversion
	}
}

func parseLogLevel(level string) log.Level {
	switch level {
	case "debug":
		return log.DEBUG
	case "info":
		return log.INFO
	case "warning":
		return log.WARNING
	case "error":
		return log.ERROR
	default:
		return log.INFO
	}
}

// Gomobile requires a main package, but we can use a dummy main
// This file will be built as a library, not executable