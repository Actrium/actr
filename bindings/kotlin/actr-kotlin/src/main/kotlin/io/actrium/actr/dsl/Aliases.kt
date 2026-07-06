/** Public aliases for the Kotlin DSL. */
package io.actrium.actr.dsl

/** Actor context exposed by workload and observer callbacks. */
typealias ActrContext = io.actrium.actr.ContextBridge

/** Incoming RPC envelope delivered to a workload. */
typealias RpcEnvelope = io.actrium.actr.RpcEnvelopeBridge

/** Workload callback interface for lifecycle and dispatch events. */
typealias Workload = io.actrium.actr.WorkloadLifecycleBridge

/** Runtime error event delivered to a workload. */
typealias ErrorEvent = io.actrium.actr.ErrorEventBridge

/** Runtime error classification. */
typealias ErrorCategory = io.actrium.actr.ErrorCategoryBridge

/** Peer-scoped transport event. */
typealias PeerEvent = io.actrium.actr.PeerEventBridge

/** Coarse WebRTC send-readiness state. */
typealias WebRtcPeerStatus = io.actrium.actr.WebRtcPeerStatusBridge

/** Credential lifecycle event. */
typealias CredentialEvent = io.actrium.actr.CredentialEventBridge

/** Mailbox backpressure event. */
typealias BackpressureEvent = io.actrium.actr.BackpressureEventBridge

/** Signaling-layer lifecycle observer. */
typealias SignalingObserver = io.actrium.actr.SignalingObserverBridge

/** WebSocket peer lifecycle observer. */
typealias WebSocketObserver = io.actrium.actr.WebSocketObserverBridge

/** WebRTC peer lifecycle observer. */
typealias WebRtcObserver = io.actrium.actr.WebRtcObserverBridge

/** Credential lifecycle observer. */
typealias CredentialObserver = io.actrium.actr.CredentialObserverBridge

/** Mailbox backpressure observer. */
typealias MailboxObserver = io.actrium.actr.MailboxObserverBridge

/** Host-side observers for a package-backed runtime. */
typealias RuntimeObservers = io.actrium.actr.RuntimeObservers

/** Dynamic workload composed from lifecycle and observer callbacks. */
typealias DynamicWorkload = io.actrium.actr.DynamicWorkload

/** Handle for platform network event callbacks. */
typealias NetworkEventHandle = io.actrium.actr.NetworkEventHandleWrapper

/** Callback interface for forwarding tracing log events to the host. */
typealias LogCallback = io.actrium.actr.LogCallback

/** Callback interface for incoming data stream chunks. */
typealias DataStreamCallback = io.actrium.actr.DataStreamCallback

/** A single audio or video frame. */
typealias MediaSample = io.actrium.actr.MediaSample

/** Callback interface for incoming media tracks. */
typealias MediaTrackCallback = io.actrium.actr.MediaTrackCallback

/** Audio or video media type. */
typealias MediaType = io.actrium.actr.MediaType

/** Opus audio encoder. */
typealias OpusEncoder = io.actrium.actr.OpusEncoder
