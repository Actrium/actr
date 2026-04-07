edition = 1

[signaling]
url = "ws://127.0.0.1:8081/signaling/ws"

[ais_endpoint]
url = "http://127.0.0.1:8081/ais"

[deployment]
realm_id = __REALM_ID__
realm_secret = "__REALM_SECRET__"

[discovery]
visible = true

[observability]
filter_level = "info"
tracing_enabled = false
tracing_endpoint = "http://localhost:4317"
tracing_service_name = "package-runtime-echo-client"

[webrtc]
force_relay = false
stun_urls = ["stun:127.0.0.1:3478"]
turn_urls = ["turn:127.0.0.1:3478"]

[acl]

[[acl.rules]]
permission = "allow"
type = "actrium:EchoService:__ECHO_SERVICE_VERSION__"
