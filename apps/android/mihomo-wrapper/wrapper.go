package mihomo

import (
	"encoding/json"
	"fmt"
	"net"
	"sort"
	"strings"
	"sync"

	"github.com/metacubex/mihomo/adapter/outboundgroup"
	"github.com/metacubex/mihomo/config"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub"
	"github.com/metacubex/mihomo/hub/executor"
	"github.com/metacubex/mihomo/log"
	"github.com/metacubex/mihomo/tunnel"
)

var (
	coreMu  sync.Mutex
	started bool
)

// proxyInfo represents a single proxy inside a group, for UI consumption.
type proxyInfo struct {
	Name   string `json:"name"`
	Type   string `json:"type"`
	Server string `json:"server"`
	Port   int    `json:"port"`
	UDP    bool   `json:"udp"`
	Alive  bool   `json:"alive"`
	Delay  int    `json:"delay"`
}

// proxyGroupInfo represents a proxy group and the proxies it can select.
type proxyGroupInfo struct {
	Name  string      `json:"name"`
	Type  string      `json:"type"`
	Nodes []proxyInfo `json:"nodes"`
	Now   string      `json:"now"`
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

	// The TUN device has to be injected into the parsed config before it is
	// applied, otherwise mihomo would try to create its own TUN interface.
	options := []hub.Option{}
	if tunFD > 0 {
		device := fmt.Sprintf("/proc/self/fd/%d", tunFD)
		options = append(options, func(cfg *config.Config) {
			cfg.General.Tun.Device = device
		})
	}

	// Initialize logging
	log.SetLevel(log.INFO)

	// hub.Parse parses the config and applies it, which starts the listeners
	// (including TUN), the DNS resolver and the proxy groups.
	if err := hub.Parse([]byte(configYAML), options...); err != nil {
		return "parse: " + err.Error()
	}

	started = true
	return ""
}

// Stop stops the mihomo core
func Stop() string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started {
		return "not running"
	}

	executor.Shutdown()
	started = false
	return ""
}

// SetMode changes the proxy mode (Rule, Global, Direct)
func SetMode(mode string) string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started {
		return "not running"
	}

	switch strings.ToLower(mode) {
	case "global":
		tunnel.SetMode(tunnel.Global)
	case "rule":
		tunnel.SetMode(tunnel.Rule)
	case "direct":
		tunnel.SetMode(tunnel.Direct)
	default:
		return "invalid mode: " + mode
	}
	return ""
}

// SelectProxy selects a proxy in a group
func SelectProxy(groupName, proxyName string) string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started {
		return "not running"
	}

	proxy, ok := tunnel.Proxies()[groupName]
	if !ok {
		return "group not found: " + groupName
	}

	// Only selector-style groups (Selector, URLTest, Fallback) accept a
	// manual choice; anything else is rejected.
	selector, ok := proxy.Adapter().(outboundgroup.SelectAble)
	if !ok {
		return "not a selector: " + groupName
	}

	if err := selector.Set(proxyName); err != nil {
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
	return started
}

// GetGroupsJSON returns every proxy group with its members as a JSON string,
// for easy JNI parsing. Returns empty string if the core is not running.
func GetGroupsJSON() string {
	coreMu.Lock()
	defer coreMu.Unlock()

	if !started {
		return ""
	}

	data, err := json.Marshal(collectGroups())
	if err != nil {
		return ""
	}
	return string(data)
}

// collectGroups snapshots the proxy-group tree the Android UI renders.
func collectGroups() []proxyGroupInfo {
	proxies := tunnel.Proxies()
	groups := make([]proxyGroupInfo, 0, len(proxies))

	for name, p := range proxies {
		// Proxy groups are the adapters that can hold other proxies; plain
		// nodes (Vmess, Trojan, ...) are not groups and are skipped.
		group, ok := p.Adapter().(outboundgroup.ProxyGroup)
		if !ok {
			continue
		}

		members := group.Proxies()
		nodes := make([]proxyInfo, 0, len(members))
		for _, member := range members {
			server, port := splitAddr(member.Addr())
			nodes = append(nodes, proxyInfo{
				Name:   member.Name(),
				Type:   member.Type().String(),
				Server: server,
				Port:   port,
				UDP:    member.SupportUDP(),
				Alive:  member.AliveForTestUrl(C.DefaultTestURL),
				Delay:  int(member.LastDelayForTestUrl(C.DefaultTestURL)),
			})
		}

		groups = append(groups, proxyGroupInfo{
			Name:  name,
			Type:  p.Type().String(),
			Nodes: nodes,
			Now:   group.Now(),
		})
	}

	// Map iteration order is random; sort so the UI does not reshuffle.
	sort.Slice(groups, func(i, j int) bool { return groups[i].Name < groups[j].Name })
	return groups
}

// splitAddr splits a mihomo "host:port" address into its two parts.
func splitAddr(addr string) (string, int) {
	if addr == "" {
		return "", 0
	}
	host, portStr, err := net.SplitHostPort(addr)
	if err != nil {
		return addr, 0
	}
	port := 0
	for _, r := range portStr {
		if r < '0' || r > '9' {
			return host, 0
		}
		port = port*10 + int(r-'0')
	}
	return host, port
}
