module github.com/akaspyrean/clashedge/apps/android/mihomo-wrapper

go 1.26.0

require (
	github.com/metacubex/mihomo v1.19.33
	golang.org/x/mobile v0.0.0-20261011034528-197770444867
)

// gomobile bind refuses to run unless golang.org/x/mobile is resolvable from
// this module, and only a tool directive keeps `go mod tidy` from dropping it.
tool golang.org/x/mobile/cmd/gobind

replace github.com/metacubex/mihomo => ../../../../mihomo-source
