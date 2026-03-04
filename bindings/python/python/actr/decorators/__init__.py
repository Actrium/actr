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
    """RPC 方法元数据"""
    name: str
    func: Callable
    route_key: str
    request_type: Optional[Type] = None
    response_type: Optional[Type] = None


@dataclass
class ServiceMetadata:
    """服务元数据"""
    service_name: str
    class_obj: Type
    rpc_methods: Dict[str, RPCMethod] = field(default_factory=dict)
    dispatcher: Optional[Any] = None
    workload_class: Optional[Type] = None


# 全局服务注册表
_service_registry: Dict[str, ServiceMetadata] = {}


def service(service_name: str):
    """
    服务装饰器，标记一个类为 Actr 服务
    
    用法:
        @actr_decorator.service("my_service.EchoService")
        class MyService:
            @actr_decorator.rpc
            async def echo(self, req: EchoRequest, ctx) -> EchoResponse:
                return EchoResponse(message=req.message)
    
    Args:
        service_name: 服务名称，格式为 "package.ServiceName"
    
    Returns:
        装饰后的类，包含自动生成的 Dispatcher 和 Workload
    """
    def decorator(cls: Type) -> Type:
        # 收集所有 RPC 方法
        rpc_methods: Dict[str, RPCMethod] = {}
        
        # 遍历类的所有方法
        for name, method in inspect.getmembers(cls, predicate=inspect.isfunction):
            if hasattr(method, '_is_rpc'):
                # 检查是否有自定义 route_key，否则使用默认格式
                route_key = getattr(method, '_custom_route_key', None) or f"{service_name}.{name}"
                
                # 从类型注解推断请求和响应类型
                try:
                    hints = get_type_hints(method)
                    # 查找参数中的 req 或 request
                    request_type = None
                    for param_name in ['req', 'request', 'request_msg']:
                        if param_name in hints:
                            request_type = hints[param_name]
                            break
                    
                    # 查找返回类型
                    response_type = hints.get('return', None)
                    
                    rpc_methods[name] = RPCMethod(
                        name=name,
                        func=method,
                        route_key=route_key,
                        request_type=request_type,
                        response_type=response_type
                    )
                except Exception as e:
                    # 如果类型推断失败，仍然注册方法，但警告用户
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
        
        # 创建服务元数据
        metadata = ServiceMetadata(
            service_name=service_name,
            class_obj=cls,
            rpc_methods=rpc_methods
        )
        
        # 生成 Dispatcher 和 Workload
        metadata.dispatcher = _generate_dispatcher(metadata)
        metadata.workload_class = _generate_workload(metadata)
        
        # 注册服务
        _service_registry[service_name] = metadata
        
        # 为类添加元数据和便捷方法
        cls._actr_metadata = metadata
        cls._actr_service_name = service_name
        
        # 添加便捷的 Workload 创建方法
        @classmethod
        def create_workload(cls_instance, *args, **kwargs):
            """便捷方法：从类创建 Workload 实例"""
            handler = cls_instance(*args, **kwargs)
            return metadata.workload_class(handler)
        
        cls.create_workload = create_workload
        
        # 添加 get_dispatcher 方法到实例
        def get_dispatcher(self):
            """返回与此服务关联的 Dispatcher"""
            return metadata.dispatcher
        
        cls.get_dispatcher = get_dispatcher
        
        return cls
    
    return decorator


def rpc(route_key: Optional[str] = None):
    """
    RPC 方法装饰器，标记一个方法为 RPC 处理函数
    
    用法:
        @actr_decorator.rpc
        async def echo(self, req: EchoRequest, ctx) -> EchoResponse:
            return EchoResponse(message=req.message)
        
        或者指定自定义 route_key:
        @actr_decorator.rpc(route_key="custom.route.key")
        async def echo(self, req: EchoRequest, ctx) -> EchoResponse:
            return EchoResponse(message=req.message)
    
    Args:
        route_key: 可选的 route_key，如果不提供则使用默认格式
    
    Returns:
        装饰后的方法
    """
    def decorator(func: Callable) -> Callable:
        # 标记为 RPC 方法
        func._is_rpc = True
        if route_key is not None:
            func._custom_route_key = route_key
        
        # 保持原函数的元数据
        @wraps(func)
        async def wrapper(*args, **kwargs):
            return await func(*args, **kwargs)
        
        # 复制属性
        wrapper._is_rpc = True
        if route_key is not None:
            wrapper._custom_route_key = route_key
        
        return wrapper
    
    # 如果直接调用 @actr_decorator.rpc（无参数），func 就是第一个参数
    if callable(route_key):
        func = route_key
        func._is_rpc = True
        return func
    
    return decorator


def _generate_dispatcher(metadata: ServiceMetadata):
    """
    自动生成 Dispatcher 类
    
    Args:
        metadata: 服务元数据
    
    Returns:
        Dispatcher 实例
    """
    class AutoDispatcher:
        """自动生成的 Dispatcher"""
        
        def __init__(self):
            self.rpc_methods = metadata.rpc_methods
        
        async def dispatch(self, workload, route_key: str, payload: bytes, ctx) -> bytes:
            """
            Dispatcher 的 dispatch 方法
            
            Args:
                workload: Workload 实例（包含 handler 属性）
                route_key: 路由键字符串
                payload: 请求 protobuf bytes
                ctx: Context 对象
            
            Returns:
                响应 protobuf bytes
            
            Raises:
                RuntimeError: 如果 route_key 未找到
                ValueError: 如果反序列化失败
            """
            # 查找匹配的 RPC 方法
            for method_name, rpc_method in self.rpc_methods.items():
                # 检查自定义 route_key 或默认 route_key
                expected_route_key = getattr(
                    rpc_method.func, '_custom_route_key', None
                ) or rpc_method.route_key
                
                if expected_route_key == route_key:
                    # 获取 Handler 实例
                    handler = getattr(workload, 'handler', workload)
                    
                    # 反序列化请求
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
                    
                    # 调用 Handler 方法
                    from actr import Context
                    if not isinstance(ctx, Context):
                        ctx = Context(ctx)
                    method = getattr(handler, method_name)
                    resp = await method(req, ctx)
                    
                    # 序列化响应
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
    自动生成 Workload 类
    
    Args:
        metadata: 服务元数据
    
    Returns:
        Workload 类
    """
    class AutoWorkload(WorkloadBase):
        """自动生成的 Workload"""
        
        def __init__(self, handler: Any):
            """
            初始化 Workload
            
            Args:
                handler: Handler 实例（用户定义的业务逻辑类）
            """
            self.handler = handler
            super().__init__(metadata.dispatcher)
        
        async def on_start(self, ctx) -> None:
            """生命周期钩子：Actor 启动时调用"""
            if hasattr(self.handler, "on_start"):
                await self.handler.on_start(ctx)
        
        async def on_stop(self, ctx) -> None:
            """生命周期钩子：Actor 停止时调用"""
            if hasattr(self.handler, "on_stop"):
                await self.handler.on_stop(ctx)
    
    return AutoWorkload


def get_service_metadata(service_name: str) -> Optional[ServiceMetadata]:
    """
    获取服务元数据
    
    Args:
        service_name: 服务名称
    
    Returns:
        服务元数据，如果未找到则返回 None
    """
    return _service_registry.get(service_name)


class ActrDecorators:
    """
    装饰器命名空间
    
    提供 @actr_decorator.service 和 @actr_decorator.rpc 装饰器
    """
    
    @staticmethod
    def service(service_name: str):
        """服务装饰器"""
        return service(service_name)
    
    @staticmethod
    def rpc(route_key: Optional[str] = None):
        """RPC 方法装饰器"""
        return rpc(route_key)
