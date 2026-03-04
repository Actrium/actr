import ActrBindings
import ActrProtocols

/// Re-export commonly used types so applications can `import Actr`.
public typealias Context = ContextBridge
public typealias RpcEnvelope = RpcEnvelopeBridge
public typealias Workload = WorkloadBridge
public typealias RpcRequest = ActrProtocols.RpcRequest
public typealias DataStream = ActrBindings.DataStream
public typealias DataStreamCallback = ActrBindings.DataStreamCallback
