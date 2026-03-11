"""
Decorator-based service definition for Actr Runtime

Provides @actr_decorator.service and @actr_decorator.rpc decorators for easy service definition.

Example:
    @actr_decorator.service("my_service.EchoService")
    class MyService:
        @actr_decorator.rpc
        async def echo(self, req: EchoRequest, ctx) -> EchoResponse:
            return EchoResponse(message=req.message)
"""

import inspect
from typing import Dict, Callable, Any, Optional, Type, get_type_hints
from dataclasses import dataclass, field
from functools import wraps
from actr.workload import WorkloadBase


@dataclass
class RPCMethod:
    """RPC method metadata"""
    name: str
    func: Callable
    route_key: str
    request_type: Optional[Type] = None
    response_type: Optional[Type] = None


@dataclass
class ServiceMetadata:
    """Service metadata"""
    service_name: str
    class_obj: Type
    rpc_methods: Dict[str, RPCMethod] = field(default_factory=dict)
    dispatcher: Optional[Any] = None
    workload_class: Optional[Type] = None


# Global service registry
_service_registry: Dict[str, ServiceMetadata] = {}


def service(service_name: str):
    """
    Service decorator, marks a class as an Actr service

    Usage:
        @actr_decorator.service("my_service.EchoService")
        class MyService:
            @actr_decorator.rpc
            async def echo(self, req: EchoRequest, ctx) -> EchoResponse:
                return EchoResponse(message=req.message)

    Args:
        service_name: Service name in format "package.ServiceName"

    Returns:
        Decorated class with auto-generated Dispatcher and Workload
    """
    def decorator(cls: Type) -> Type:
        # Collect all RPC methods
        rpc_methods: Dict[str, RPCMethod] = {}

        # Iterate through all methods in the class
        for name, method in inspect.getmembers(cls, predicate=inspect.isfunction):
            if hasattr(method, '_is_rpc'):
                # Check if there's a custom route_key, otherwise use default format
                route_key = getattr(method, '_custom_route_key', None) or f"{service_name}.{name}"

                # Infer request and response types from type annotations
                try:
                    hints = get_type_hints(method)
                    # Find req or request in parameters
                    request_type = None
                    for param_name in ['req', 'request', 'request_msg']:
                        if param_name in hints:
                            request_type = hints[param_name]
                            break

                    # Find return type
                    response_type = hints.get('return', None)

                    rpc_methods[name] = RPCMethod(
                        name=name,
                        func=method,
                        route_key=route_key,
                        request_type=request_type,
                        response_type=response_type
                    )
                except Exception as e:
                    # If type inference fails, still register method but warn user
                    import warnings
                    warnings.warn(
                        f"Failed to infer types for RPC method {name} in {cls.__name__}: {e}. "
                        "Please ensure type hints are correct.",
                        RuntimeWarning
                    )
                    rpc_methods[name] = RPCMethod(
                        name=name,
                        func=method,
                        route_key=route_key,
                        request_type=None,
                        response_type=None
                    )

        # Create service metadata
        metadata = ServiceMetadata(
            service_name=service_name,
            class_obj=cls,
            rpc_methods=rpc_methods
        )

        # Generate Dispatcher and Workload
        metadata.dispatcher = _generate_dispatcher(metadata)
        metadata.workload_class = _generate_workload(metadata)

        # Register service
        _service_registry[service_name] = metadata

        # Add metadata and convenience methods to class
        cls._actr_metadata = metadata
        cls._actr_service_name = service_name

        # Add convenience Workload creation method
        @classmethod
        def create_workload(cls_instance, *args, **kwargs):
            """Convenience method: create Workload instance from class"""
            handler = cls_instance(*args, **kwargs)
            return metadata.workload_class(handler)

        cls.create_workload = create_workload

        # Add get_dispatcher method to instance
        def get_dispatcher(self):
            """Return Dispatcher associated with this service"""
            return metadata.dispatcher

        cls.get_dispatcher = get_dispatcher

        return cls

    return decorator


def rpc(route_key: Optional[str] = None):
    """
    RPC method decorator, marks a method as RPC handler

    Usage:
        @actr_decorator.rpc
        async def echo(self, req: EchoRequest, ctx) -> EchoResponse:
            return EchoResponse(message=req.message)

        Or specify custom route_key:
        @actr_decorator.rpc(route_key="custom.route.key")
        async def echo(self, req: EchoRequest, ctx) -> EchoResponse:
            return EchoResponse(message=req.message)

    Args:
        route_key: Optional route_key, uses default format if not provided

    Returns:
        Decorated method
    """
    def decorator(func: Callable) -> Callable:
        # Mark as RPC method
        func._is_rpc = True
        if route_key is not None:
            func._custom_route_key = route_key

        # Preserve original function metadata
        @wraps(func)
        async def wrapper(*args, **kwargs):
            return await func(*args, **kwargs)

        # Copy attributes
        wrapper._is_rpc = True
        if route_key is not None:
            wrapper._custom_route_key = route_key

        return wrapper

    # If called directly @actr_decorator.rpc (no arguments), func is first parameter
    if callable(route_key):
        func = route_key
        func._is_rpc = True
        return func

    return decorator


def _generate_dispatcher(metadata: ServiceMetadata):
    """
    Auto-generate Dispatcher class

    Args:
        metadata: Service metadata

    Returns:
        Dispatcher instance
    """
    class AutoDispatcher:
        """Auto-generated Dispatcher"""

        def __init__(self):
            self.rpc_methods = metadata.rpc_methods

        async def dispatch(self, workload, route_key: str, payload: bytes, ctx) -> bytes:
            """
            Dispatcher dispatch method

            Args:
                workload: Workload instance (contains handler attribute)
                route_key: Route key string
                payload: Request protobuf bytes
                ctx: Context object

            Returns:
                Response protobuf bytes

            Raises:
                RuntimeError: If route_key not found
                ValueError: If deserialization fails
            """
            # Find matching RPC method
            for method_name, rpc_method in self.rpc_methods.items():
                # Check custom route_key or default route_key
                expected_route_key = getattr(
                    rpc_method.func, '_custom_route_key', None
                ) or rpc_method.route_key

                if expected_route_key == route_key:
                    # Get Handler instance
                    handler = getattr(workload, 'handler', workload)

                    # Deserialize request
                    if rpc_method.request_type is None:
                        raise ValueError(
                            f"Cannot deserialize request for {route_key}: "
                            "request type not specified. Please add type hints."
                        )

                    try:
                        req = rpc_method.request_type.FromString(payload)
                    except Exception as e:
                        raise ValueError(
                            f"Failed to deserialize request for {route_key}: {e}"
                        ) from e

                    # Call Handler method
                    from actr import Context
                    if not isinstance(ctx, Context):
                        ctx = Context(ctx)
                    method = getattr(handler, method_name)
                    resp = await method(req, ctx)

                    # Serialize response
                    if not hasattr(resp, 'SerializeToString'):
                        raise ValueError(
                            f"Response from {route_key} is not a protobuf message. "
                            f"Got type: {type(resp)}"
                        )

                    return resp.SerializeToString()

            raise RuntimeError(f"Unknown route_key: {route_key}")

    return AutoDispatcher()


def _generate_workload(metadata: ServiceMetadata):
    """
    Auto-generate Workload class

    Args:
        metadata: Service metadata

    Returns:
        Workload class
    """
    class AutoWorkload(WorkloadBase):
        """Auto-generated Workload"""

        def __init__(self, handler: Any):
            """
            Initialize Workload

            Args:
                handler: Handler instance (user-defined business logic class)
            """
            self.handler = handler
            super().__init__(metadata.dispatcher)

        async def on_start(self, ctx) -> None:
            """Lifecycle hook: called when Actor starts"""
            if hasattr(self.handler, "on_start"):
                await self.handler.on_start(ctx)

        async def on_stop(self, ctx) -> None:
            """Lifecycle hook: called when Actor stops"""
            if hasattr(self.handler, "on_stop"):
                await self.handler.on_stop(ctx)

    return AutoWorkload


def get_service_metadata(service_name: str) -> Optional[ServiceMetadata]:
    """
    Get service metadata

    Args:
        service_name: Service name

    Returns:
        Service metadata, or None if not found
    """
    return _service_registry.get(service_name)


class ActrDecorators:
    """
    Decorator namespace

    Provides @actr_decorator.service and @actr_decorator.rpc decorators
    """

    @staticmethod
    def service(service_name: str):
        """Service decorator"""
        return service(service_name)

    @staticmethod
    def rpc(route_key: Optional[str] = None):
        """RPC method decorator"""
        return rpc(route_key)
