"""
Type stubs for actr_raw Rust extension module.

This file provides type hints for the Rust-based actr_raw module,
enabling IDE autocomplete and type checking.
"""

from typing import Any, Callable, Coroutine, Optional
from enum import Enum

# Exception types
class ActrRuntimeError(Exception):
    """Runtime error from the Actr system."""
    ...

class ActrTransportError(Exception):
    """Transport error during communication."""
    ...

class ActrDecodeError(Exception):
    """Error decoding a message."""
    ...

class ActrUnknownRoute(Exception):
    """Unknown route key error."""
    ...

class ActrGateNotInitialized(Exception):
    """Gate not initialized error."""
    ...

# Payload type enum
class PayloadType(Enum):
    """Payload transmission type."""
    RpcReliable = ...
    RpcSignal = ...
    StreamReliable = ...
    StreamLatencyFirst = ...

# ActrId binding
class ActrId:
    """Actor ID binding type."""
    @staticmethod
    def from_bytes(bytes: bytes) -> "ActrId": ...
    def to_bytes(self) -> bytes: ...

# ActrType binding
class ActrType:
    """Actor type binding."""
    def __init__(self, manufacturer: str, name: str) -> None: ...
    def to_bytes(self) -> bytes: ...
    def manufacturer(self) -> str: ...
    def name(self) -> str: ...

# Dest class for specifying message targets
class Dest:
    """Destination identifier for messages."""
    
    @staticmethod
    def shell() -> "Dest":
        """Create a Dest targeting the shell (external caller)."""
        ...
    
    @staticmethod
    def local() -> "Dest":
        """Create a Dest targeting the local workload."""
        ...
    
    @staticmethod
    def actor(actr_id: ActrId) -> "Dest":
        """
        Create a Dest targeting a specific actor by ID.
        
        Args:
            actr_id: The actor ID (protobuf ActrId object)
        
        Returns:
            Dest instance targeting the specified actor
        """
        ...
    
    def is_shell(self) -> bool:
        """Check if this Dest targets the shell."""
        ...
    
    def is_local(self) -> bool:
        """Check if this Dest targets the local workload."""
        ...
    
    def is_actor(self) -> bool:
        """Check if this Dest targets a specific actor."""
        ...
    
    def as_actor_id(self) -> Optional[ActrId]:
        """Get the actor ID if this Dest targets an actor, otherwise None."""
        ...

# DataStream class for streaming data
class DataStream:
    """Wrapper for DataStream."""
    
    def __init__(
        self,
        stream_id: str,
        sequence: int,
        payload: bytes,
        timestamp_ms: Optional[int] = ...,
    ) -> None:
        """
        Create a DataStream from fields.
        
        Args:
            stream_id: Stream identifier
            sequence: Sequence number
            payload: Payload bytes
            timestamp_ms: Optional timestamp in milliseconds
        """
        ...
    
    @staticmethod
    def from_bytes(bytes: bytes) -> "DataStream":
        """Create a DataStream from serialized bytes."""
        ...
    
    def to_bytes(self) -> bytes:
        """Serialize to bytes."""
        ...
    
    def stream_id(self) -> str:
        """Get the stream ID."""
        ...
    
    def sequence(self) -> int:
        """Get the sequence number."""
        ...
    
    def payload(self) -> bytes:
        """Get the payload bytes."""
        ...
    
    def timestamp_ms(self) -> Optional[int]:
        """Get the optional timestamp in milliseconds."""
        ...

# ActrSystem class
class ActrSystem:
    """Main entry point for creating an Actr system."""
    
    @staticmethod
    async def from_toml(path: str) -> "ActrSystem":
        """
        Create an ActrSystem from a TOML configuration file.
        
        Args:
            path: Path to the TOML configuration file
        
        Returns:
            ActrSystem instance
        """
        ...
    
    def attach(self, workload: Any) -> "ActrNode":
        """
        Attach a workload to the system.
        
        Args:
            workload: A workload instance with on_start, dispatch, on_stop methods
        
        Returns:
            ActrNode instance
        """
        ...

# ActrNode class
class ActrNode:
    """Represents an attached but not yet started actor node."""
    
    async def start(self) -> "ActrRef":
        """
        Start the node.
        
        Returns:
            ActrRef instance for interacting with the running actor
        """
        ...
    
    async def try_start(self) -> "ActrRef":
        """
        Try to start the node.
        
        Returns:
            ActrRef instance for interacting with the running actor
        """
        ...

# ActrRef class
class ActrRef:
    """Reference to a running actor, used for external interaction."""
    
    def actor_id(self) -> Any:
        """Get the actor's ID."""
        ...
    
    def shutdown(self) -> None:
        """Trigger actor shutdown."""
        ...
    
    async def wait_for_shutdown(self) -> None:
        """Wait for the actor to fully shutdown."""
        ...
    
    async def wait_for_ctrl_c_and_shutdown(self) -> None:
        """Wait for Ctrl+C signal, then shutdown."""
        ...
    
    async def call(
        self,
        route_key: str,
        request: bytes,
        timeout_ms: int = 30000,
        payload_type: PayloadType = PayloadType.RpcReliable,
    ) -> bytes:
        """
        Call an RPC method on the actor (Shell → Workload).
        
        Args:
            route_key: Route key string
            request: Request payload bytes
            timeout_ms: Timeout in milliseconds
            payload_type: Payload transmission type
        
        Returns:
            Response bytes
        """
        ...
    
    async def tell(
        self,
        route_key: str,
        message: bytes,
        payload_type: PayloadType = PayloadType.RpcReliable,
    ) -> None:
        """
        Send a one-way message to the actor (fire-and-forget).
        
        Args:
            route_key: Route key string
            message: Message payload bytes
            payload_type: Payload transmission type
        """
        ...

# Context class
class Context:
    """Context provided to workload methods for actor operations."""
    
    def self_id(self) -> ActrId:
        """Get the current actor's ID."""
        ...
    
    def caller_id(self) -> Optional[ActrId]:
        """Get the caller's actor ID, if available."""
        ...
    
    def request_id(self) -> str:
        """Get the current request ID."""
        ...
    
    async def discover_route_candidate(self, actr_type: ActrType) -> ActrId:
        """
        Discover a route candidate by actor type.
        
        Args:
            actr_type: ActrType binding
        
        Returns:
            Actor ID of a discovered candidate
        """
        ...
    
    async def call_raw(
        self,
        target: Dest,
        route_key: str,
        request: bytes,
        timeout_ms: int = 30000,
        payload_type: PayloadType = PayloadType.RpcReliable,
    ) -> bytes:
        """
        Execute a request/response RPC call.
        
        Args:
            target: Destination (use Dest.actor(actor_id))
            route_key: Route key string
            request: Request payload bytes
            timeout_ms: Timeout in milliseconds
            payload_type: Payload transmission type
        
        Returns:
            Response bytes
        """
        ...
    
    async def tell_raw(
        self,
        target: Dest,
        route_key: str,
        message: bytes,
        payload_type: PayloadType = PayloadType.RpcReliable,
    ) -> None:
        """
        Execute a fire-and-forget message send.
        
        Args:
            target: Destination (use Dest.actor(actor_id))
            route_key: Route key string
            message: Message payload bytes
            payload_type: Payload transmission type
        """
        ...
    
    async def register_stream(
        self,
        stream_id: str,
        callback: Callable[[DataStream, ActrId], Coroutine[Any, Any, None]],
    ) -> None:
        """
        Register a callback for receiving stream data.
        
        Args:
            stream_id: Stream identifier
            callback: Async callback function(data_stream, sender_id)
        """
        ...
    
    async def unregister_stream(self, stream_id: str) -> None:
        """
        Unregister a stream callback.
        
        Args:
            stream_id: Stream identifier
        """
        ...
    
    async def send_data_stream(self, target: Dest, data_stream: DataStream) -> None:
        """
        Send a data stream chunk.
        
        Args:
            target: Destination (use Dest.actor(actor_id))
            data_stream: DataStream wrapper object
        """
        ...

# Re-export all types
__all__ = [
    "ActrSystem",
    "ActrNode",
    "ActrRef",
    "Context",
    "ActrId",
    "ActrType",
    "Dest",
    "PayloadType",
    "DataStream",
    "ActrRuntimeError",
    "ActrTransportError",
    "ActrDecodeError",
    "ActrUnknownRoute",
    "ActrGateNotInitialized",
]
