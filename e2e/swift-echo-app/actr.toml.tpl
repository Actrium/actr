# manifest.toml — EchoApp Linked Runtime 配置
#
# 设计意图:
#   EchoApp 是 linked 模式 (ActrNode.linked), 不走 actr build。
#   此文件直接由 ActrNode.linked() 读取作为运行时配置,
#   不需要 [package]/[binary]/[build] 等打包段。
#
#   actr type 在 Swift 代码中定义:
#     ActrType(manufacturer: "acme", name: "EchoApp", version: "0.1.0")

[signaling]
url = "ws://__HOST__:__HTTP_PORT__/signaling/ws"

[ais_endpoint]
url = "http://__HOST__:__HTTP_PORT__/ais"

[deployment]
# Replace this with the REALM_ID returned by actrix CreateRealm/Admin UI.
realm_id = __REALM_ID__
realm_secret = "__REALM_SECRET__"

[discovery]
visible = true

[observability]
filter_level = "info"
tracing_enabled = true
tracing_endpoint = "http://localhost:4317"
tracing_service_name = "echo-app-ios"

[webrtc]
force_relay = false
stun_urls = ["stun:__HOST__:__ICE_PORT__"]
turn_urls = ["turn:__HOST__:__ICE_PORT__"]

[acl]

[[acl.rules]]
permission = "allow"
type = "actrium:EchoService:1.0.0"
