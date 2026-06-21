edition = 1

[signaling]
url = "ws://127.0.0.1:__HTTP_PORT__/signaling/ws"

[ais_endpoint]
url = "http://127.0.0.1:__HTTP_PORT__/ais"

[deployment]
realm_id = __REALM_ID__

[discovery]
visible = true

[observability]
filter_level = "info"
tracing_enabled = false

[webrtc]
force_relay = false
stun_urls = ["stun:127.0.0.1:__ICE_PORT__"]
turn_urls = ["turn:127.0.0.1:__ICE_PORT__"]

[acl]

# Each client driver registers under its own actr_type via
# `Node::from_config_with_package` + a per-driver `manifest.toml`, so the
# allow-list enumerates them explicitly. The streaming scenarios run only
# the Rust driver today; TS/other drivers append a rule here when added.
[[acl.rules]]
permission = "allow"
type = "polyglot:RustDriver:0.1.0"

[[acl.rules]]
permission = "allow"
type = "polyglot:TsDriver:0.1.0"

[[trust]]
kind = "static"
pubkey_b64 = "__MFR_PUBKEY__"
