from __future__ import annotations

from typing import Any, Callable, Coroutine, Optional, Type, TypeVar

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

__version__: str

T = TypeVar("T")
R = TypeVar("R")


class ActrSystem:
    def __init__(self, rust_system: RustActrSystem) -> None: ...
    @staticmethod
    async def from_toml(path: str) -> ActrSystem: ...
    def attach(self, workload: Any) -> ActrNode: ...


class ActrNode:
    def __init__(self, rust_node: RustActrNode) -> None: ...
    async def start(self) -> ActrRef: ...
    async def try_start(self) -> ActrRef: ...


class ActrRef:
    def __init__(self, rust_ref: RustActrRef) -> None: ...
    def actor_id(self) -> RustActrId: ...
    async def call(
        self,
        route_key: str,
        request: Any,
        timeout_ms: int = 30000,
        payload_type: Optional[RustPayloadType] = ...,
    ) -> bytes: ...
    async def tell(
        self,
        route_key: str,
        message: Any,
        payload_type: Optional[RustPayloadType] = ...,
    ) -> None: ...
    def shutdown(self) -> None: ...
    async def wait_for_shutdown(self) -> None: ...
    async def wait_for_ctrl_c_and_shutdown(self) -> None: ...


class Context:
    def __init__(self, rust_ctx: Any) -> None: ...
    def self_id(self) -> RustActrId: ...
    def caller_id(self) -> Optional[RustActrId]: ...
    def request_id(self) -> str: ...
    async def discover(self, actr_type: RustActrType) -> RustActrId: ...
    async def call(
        self,
        target: RustDest,
        route_key: str,
        request: Any,
        timeout_ms: int = 30000,
        payload_type: Optional[RustPayloadType] = ...,
    ) -> bytes: ...
    async def tell(
        self,
        target: RustDest,
        route_key: str,
        message: Any,
        payload_type: Optional[RustPayloadType] = ...,
    ) -> None: ...
    async def register_stream(
        self,
        stream_id: str,
        callback: Callable[[RustDataStream, RustActrId], Coroutine[Any, Any, None]],
    ) -> None: ...
    async def unregister_stream(self, stream_id: str) -> None: ...
    async def send_stream(
        self,
        target: RustDest,
        data_stream: RustDataStream,
        payload_type: Optional[RustPayloadType] = ...,
    ) -> None: ...


def service(service_name: str) -> Callable[[Type[T]], Type[T]]: ...

def rpc(route_key: Optional[str] = ...) -> Callable[[Callable[..., Any]], Callable[..., Any]]: ...


class ActrDecorators:
    @staticmethod
    def service(service_name: str) -> Callable[[Type[T]], Type[T]]: ...
    @staticmethod
    def rpc(route_key: Optional[str] = ...) -> Callable[[Callable[..., Any]], Callable[..., Any]]: ...

Dest = RustDest
PayloadType = RustPayloadType
DataStream = RustDataStream
ActrId = RustActrId
ActrType = RustActrType

from . import actr_raw
from .workload import WorkloadBase
from .decorators import service, rpc, ActrDecorators

actr_decorator: ActrDecorators

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
    "service",
    "rpc",
    "ActrDecorators",
    "actr_decorator",
    "actr_raw",
    "WorkloadBase",
]
