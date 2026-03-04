"""
Actr Python SDK

This module provides Python bindings for actr-runtime, including:
- High-level Pythonic API: Python-friendly wrappers (root package)
- Rust Binding: Direct Rust bindings (in .actr_raw submodule, imports from actr_raw)

Recommended usage:
    from actr import ActrSystem, WorkloadBase
    from generated import my_service_pb2, my_service_actor

    class MyHandler(my_service_actor.MyServiceHandler):
        async def echo(self, req: my_service_pb2.EchoRequest, ctx):
            return my_service_pb2.EchoResponse(message=req.message)

    class MyWorkload(WorkloadBase):
        def __init__(self, handler: MyHandler):
            self.handler = handler
            super().__init__(my_service_actor.MyServiceDispatcher())

    system = await ActrSystem.from_toml("Actr.toml")
    node = system.attach(MyWorkload(MyHandler()))
    ref = await node.start()
"""

from importlib import metadata as _metadata
from typing import Optional, Union, TypeVar, Any

try:
    __version__ = _metadata.version("actr")
except _metadata.PackageNotFoundError:
    __version__ = "0.0.0"
from .actr_raw import (
    ActrSystem as RustActrSystem,
    ActrNode as RustActrNode,
    ActrRef as RustActrRef,
    ActrId as RustActrId,
    ActrType as RustActrType,
    Dest as RustDest,
    PayloadType as RustPayloadType,
    DataStream as RustDataStream,
    ActrRuntimeError,
    ActrTransportError,
    ActrDecodeError,
    ActrUnknownRoute,
    ActrGateNotInitialized,
)
from .workload import WorkloadBase  



class ActrSystem:
    """
    High-level ActrSystem wrapper
    
    Provides a Pythonic interface for creating and managing Actor systems.
    """
    
    def __init__(self, rust_system):
        """Initialize with Rust ActrSystem"""
        self._rust = rust_system
    
    @staticmethod
    async def from_toml(path: str):
        """
        Create ActrSystem from TOML configuration file
        
        Args:
            path: Path to TOML configuration file
        
        Returns:
            ActrSystem instance
        
        Raises:
            RuntimeError: If Rust extension is not available
            ActrRuntimeError: If system creation fails
        """
        if RustActrSystem is None:
            raise RuntimeError("Rust ActrSystem not available. Make sure Rust extension is built.")
        rust_system = await RustActrSystem.from_toml(path)
        return ActrSystem(rust_system)
    
    def attach(self, workload):
        """
        Attach workload to system
        
        Args:
            workload: Workload instance
        
        Returns:
            ActrNode instance
        """
        rust_node = self._rust.attach(workload)
        return ActrNode(rust_node)


class ActrNode:
    """
    High-level ActrNode wrapper
    
    Provides Pythonic interface for Actor nodes.
    """
    
    def __init__(self, rust_node):
        """Initialize with Rust ActrNode"""
        self._rust = rust_node
    
    async def start(self):
        """
        Start the node
        
        Returns:
            ActrRef instance
        
        Raises:
            ActrRuntimeError: If node start fails
        """
        rust_ref = await self._rust.start()
        return ActrRef(rust_ref)
    
    async def try_start(self):
        """
        Try to start the node
        
        Returns:
            ActrRef instance
        
        Raises:
            ActrRuntimeError: If node start fails
        """
        rust_ref = await self._rust.try_start()
        return ActrRef(rust_ref)


class ActrRef:
    """
    High-level ActrRef wrapper
    
    Provides Pythonic methods for interacting with running Actors.
    """
    
    def __init__(self, rust_ref):
        """Initialize with Rust ActrRef"""
        self._rust = rust_ref
    
    def actor_id(self) -> "ActrId":
        """Get Actor ID"""
        return self._rust.actor_id()
    
    async def call(
        self,
        route_key: str,
        request,
        timeout_ms: int = 30000,
        payload_type: "PayloadType" = None
    ):
        """
        Call Actor method (Shell → Workload RPC)
        
        Args:
            route_key: Route key string
            request: Request protobuf object (not bytes)
            timeout_ms: Timeout in milliseconds
            payload_type: Payload transmission type
        
        Returns:
            Response bytes
        
        Raises:
            ActrRuntimeError: If RPC call fails
            ValueError: If request is not a protobuf message
        """
        if payload_type is None:
            payload_type = RustPayloadType.RpcReliable
        
        # Serialize request
        if not hasattr(request, 'SerializeToString'):
            raise ValueError(f"Request must be a protobuf message, got {type(request)}")
        request_bytes = request.SerializeToString()
        
        # Call Rust method (now raises exception directly on error)
        return await self._rust.call(
            route_key,
            request_bytes,
            timeout_ms,
            payload_type
        )
    
    async def tell(
        self,
        route_key: str,
        message,
        payload_type: "PayloadType" = None
    ):
        """
        Send one-way message (Shell → Workload, fire-and-forget)
        
        Args:
            route_key: Route key string
            message: Message protobuf object (not bytes)
            payload_type: Payload transmission type
        
        Raises:
            ActrRuntimeError: If message send fails
            ValueError: If message is not a protobuf message
        """
        if payload_type is None:
            payload_type = RustPayloadType.RpcReliable
        
        # Serialize message
        if not hasattr(message, 'SerializeToString'):
            raise ValueError(f"Message must be a protobuf message, got {type(message)}")
        message_bytes = message.SerializeToString()
        
        # Call Rust method (now raises exception directly on error)
        await self._rust.tell(
            route_key,
            message_bytes,
            payload_type
        )
    
    def shutdown(self):
        """Trigger Actor shutdown"""
        self._rust.shutdown()
    
    async def wait_for_shutdown(self):
        """Wait for Actor to fully shutdown"""
        await self._rust.wait_for_shutdown()
    
    async def wait_for_ctrl_c_and_shutdown(self):
        """Wait for Ctrl+C signal, then shutdown"""
        await self._rust.wait_for_ctrl_c_and_shutdown()


class Context:
    """
    High-level Context wrapper
    
    Provides Pythonic methods for Actor context operations.
    """
    
    def __init__(self, rust_ctx):
        """Initialize with Rust Context"""
        self._rust = rust_ctx
    
    def self_id(self) -> "ActrId":
        """Get self Actor ID"""
        return self._rust.self_id()
    
    def caller_id(self) -> Optional["ActrId"]:
        """Get caller Actor ID"""
        return self._rust.caller_id()
    
    def request_id(self) -> str:
        """Get current request ID"""
        return self._rust.request_id()
    
    async def discover(self, actr_type: "ActrType"):
        """
        Discover route candidate
        
        Args:
            actr_type: ActrType binding object
        
        Returns:
            Actor id binding object
        
        Raises:
            ActrRuntimeError: If discovery fails
        """
        return await self._rust.discover_route_candidate(actr_type)
    
    async def call(
        self,
        target: "Dest",
        route_key: str,
        request,
        timeout_ms: int = 30000,
        payload_type: "PayloadType" = None
    ):
        """
        Execute request/response RPC call
        
        Args:
            target: Dest binding object
            route_key: Route key string
            request: Request protobuf object (not bytes)
            timeout_ms: Timeout in milliseconds
            payload_type: Payload transmission type
        
        Returns:
            Response bytes
        
        Raises:
            ActrRuntimeError: If RPC call fails
            ValueError: If request is not a protobuf message
        """
        if payload_type is None:
            payload_type = RustPayloadType.RpcReliable
        
        # Serialize request
        if not hasattr(request, 'SerializeToString'):
            raise ValueError(f"Request must be a protobuf message, got {type(request)}")
        request_bytes = request.SerializeToString()
        
        # Call Rust method (now raises exception directly on error)
        return await self._rust.call_raw(
            target,
            route_key,
            request_bytes,
            timeout_ms,
            payload_type
        )
    
    async def tell(
        self,
        target: "Dest",
        route_key: str,
        message,
        payload_type: "PayloadType" = None
    ):
        """
        Execute fire-and-forget message RPC call
        
        Args:
            target: Dest binding object
            route_key: Route key string
            message: Message protobuf object (not bytes)
            payload_type: Payload transmission type
        
        Raises:
            ActrRuntimeError: If message send fails
            ValueError: If message is not a protobuf message
        """
        if payload_type is None:
            payload_type = RustPayloadType.RpcReliable
        
        # Serialize message
        if not hasattr(message, 'SerializeToString'):
            raise ValueError(f"Message must be a protobuf message, got {type(message)}")
        message_bytes = message.SerializeToString()
        
        # Call Rust method (now raises exception directly on error)
        await self._rust.tell_raw(
            target,
            route_key,
            message_bytes,
            payload_type
        )
    
    async def register_stream(self, stream_id: str, callback):
        """Register DataStream callback (callback receives DataStream and ActrId)"""
        async def _wrapped(data_stream, sender_id):
            if not isinstance(sender_id, ActrId) and hasattr(
                sender_id, "SerializeToString"
            ):
                sender_id = ActrId.from_bytes(sender_id.SerializeToString())
            return await callback(data_stream, sender_id)

        await self._rust.register_stream(stream_id, _wrapped)
    
    async def unregister_stream(self, stream_id: str):
        """Unregister DataStream"""
        await self._rust.unregister_stream(stream_id)
    
    async def send_stream(
        self,
        target: "Dest",
        data_stream: "DataStream",
        payload_type: "PayloadType" = None
    ):
        """
        Send DataStream
        
        Args:
            target: Dest wrapper object
            data_stream: DataStream object
            payload_type: Payload transmission type
        
        Raises:
            ActrRuntimeError: If send fails
        """
        if payload_type is None:
            payload_type = RustPayloadType.StreamReliable
        
        # Convert protobuf to wrapper if needed
        if not isinstance(data_stream, RustDataStream):
            data_stream = RustDataStream(data_stream)
        
        # Call Rust method (now raises exception directly on error)
        await self._rust.send_data_stream(target, data_stream)


# Re-export types for convenience
Dest = RustDest
PayloadType = RustPayloadType
DataStream = RustDataStream
ActrId = RustActrId
ActrType = RustActrType

# Import decorators
from .decorators import service, rpc, ActrDecorators

# Create decorator namespace
actr_decorator = ActrDecorators()

# Re-export Rust binding module for advanced users who need direct access
# Usage: from actr.actr_raw import ActrSystem, ActrRef, etc.
from . import actr_raw

__all__ = [
    "__version__",
    # High-level Pythonic API (recommended, root package)
    "ActrSystem",
    "ActrNode",
    "ActrRef",
    "Context",
    "ActrId",
    "ActrType",
    "Dest",
    "PayloadType",
    "DataStream",
    # Decorators (recommended)
    "actr_decorator",
    "service",
    "rpc",
    # Exceptions
    "ActrRuntimeError",
    "ActrTransportError",
    "ActrDecodeError",
    "ActrUnknownRoute",
    "ActrGateNotInitialized",
    # Submodules (for direct access)
    "actr_raw",
    "decorators",
]
