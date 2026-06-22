# 专有名词解释

## AIS
**AId Issue Service**: Actrix 框架提供的一个 HTTP 服务，负责处理身份凭证发放。

## KS
**Key Server**: 一个纯内部服务，专职生成和管理椭圆曲线公私钥。它使用预共享密钥（PSK）结合 Nonce 机制来对访问进行认证，以保证安全性。

## Signaling
**Signaling Service**: WebRTC 信令服务，用于在 WebRTC 连接建立之前交换元数据。

## STUN
**Session Traversal Utilities for NAT**: NAT 会话穿越应用程序，用于帮助设备发现其公共网络地址。

## TURN
**Traversal Using Relays around NAT**: 在 STUN 不足以建立连接时，通过中继服务器转发 WebRTC 流量的服务。

## Managed Service
一个内部 HTTP 服务，可能用于管理或监控目的。