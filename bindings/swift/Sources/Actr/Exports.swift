import ActrBindings

/// Re-export commonly used types so applications can `import Actr`.
public typealias Context = ContextBridge
public typealias RpcEnvelope = RpcEnvelopeBridge
public typealias Workload = WorkloadLifecycleBridge
public typealias DynamicWorkload = ActrBindings.DynamicWorkload
public typealias DataStream = ActrBindings.DataStream
public typealias DataStreamCallback = ActrBindings.DataStreamCallback
public typealias ErrorEventBridge = ActrBindings.ErrorEventBridge
public typealias MediaSample = ActrBindings.MediaSample
public typealias MediaType = ActrBindings.MediaType
public typealias MediaTrackCallback = ActrBindings.MediaTrackCallback
public typealias OpusEncoder = ActrBindings.OpusEncoder
