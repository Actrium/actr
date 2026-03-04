"""
Concrete strategy implementations for different code generation scenarios.
"""

from __future__ import annotations

import sys
from typing import List

from .strategies import (
    GenerationStrategy,
    GenerationContext,
    GeneratedFile,
)


class EmptyLocalWorkloadStrategy(GenerationStrategy):
    """
    Strategy for empty local proto files (pure client scenario).
    
    Handles files that:
    - Are NOT remote dependencies
    - Have NO service definitions
    - Are explicitly marked as local files
    
    Generates:
    - Empty workload with remote service proxies
    """
    
    @property
    def priority(self) -> int:
        return 1  # Highest priority
    
    @property
    def name(self) -> str:
        return "EmptyLocalWorkload"
    
    def can_handle(self, file_desc, context: GenerationContext) -> bool:
        return (
            not context.is_remote(file_desc) and
            not context.has_services(file_desc) and
            context.is_local(file_desc)
        )
    
    def generate(
        self, 
        file_desc, 
        context: GenerationContext
    ) -> List[GeneratedFile]:
        from . import generators
        
        output = generators.generate_empty_local_workload(
            file_desc.package,
            file_desc.name,
            context.remote_services,
        )
        
        context.has_local_workload = True
        
        print(
            f"INFO: Generated empty local workload for '{file_desc.name}'",
            file=sys.stderr
        )
        
        return [GeneratedFile(name=output["name"], content=output["content"])]


class RemoteServiceStrategy(GenerationStrategy):
    """
    Strategy for remote service dependencies.
    
    Handles files that:
    - Are remote dependencies (in RemoteFileMapping)
    - Have service definitions
    
    Generates:
    - RPC request extensions only (client-side interfaces)
    """
    
    @property
    def priority(self) -> int:
        return 2
    
    @property
    def name(self) -> str:
        return "RemoteService"
    
    def can_handle(self, file_desc, context: GenerationContext) -> bool:
        return context.is_remote(file_desc)
    
    def generate(
        self, 
        file_desc, 
        context: GenerationContext
    ) -> List[GeneratedFile]:
        from . import generators
        
        generated_files = []
        
        for service in file_desc.service:
            output = generators.generate_remote_extensions_only(
                file_desc.package,
                file_desc.name,
                service.name,
                service.method,
            )
            
            generated_files.append(
                GeneratedFile(name=output["name"], content=output["content"])
            )
            
            print(
                f"INFO: Generated remote extensions for service '{service.name}' "
                f"in file '{file_desc.name}'",
                file=sys.stderr
            )
        
        return generated_files


class LocalServiceStrategy(GenerationStrategy):
    """
    Strategy for local service implementations (server scenario).
    
    Handles files that:
    - Are NOT remote dependencies
    - Have service definitions
    
    Generates:
    - Complete actor code (Handler, Dispatcher, Workload)
    - RPC request extensions
    - Remote service proxies (if dependencies exist)
    """
    
    @property
    def priority(self) -> int:
        return 3
    
    @property
    def name(self) -> str:
        return "LocalService"
    
    def can_handle(self, file_desc, context: GenerationContext) -> bool:
        return (
            not context.is_remote(file_desc) and
            context.has_services(file_desc)
        )
    
    def generate(
        self, 
        file_desc, 
        context: GenerationContext
    ) -> List[GeneratedFile]:
        from . import generators
        
        generated_files = []
        
        for service in file_desc.service:
            output = generators.generate_local_actor_code(
                package_name=file_desc.package,
                proto_name=file_desc.name,
                service_name=service.name,
                methods=service.method,
                remote_services=context.remote_services,
            )
            
            # Use package_name (which now contains project name) for filename
            # e.g., package "echo_app" -> "echo_app_service_actor.py"
            # This ensures consistency with template expectations
            package_snake = generators.to_snake_case(file_desc.package) if file_desc.package else generators.to_snake_case(service.name)
            file_name = f"{package_snake}_service_actor.py"
            generated_files.append(GeneratedFile(name=file_name, content=output))
            
            print(
                f"INFO: Generated local actor code for service '{service.name}' "
                f"in file '{file_desc.name}' as '{file_name}'",
                file=sys.stderr
            )
        
        context.has_local_workload = True
        
        return generated_files


class DefaultClientWorkloadStrategy(GenerationStrategy):
    """
    Strategy for generating a default client workload.
    
    This is a special strategy that doesn't handle individual files,
    but generates a fallback workload when:
    - There are remote services
    - No local workload was generated
    
    This strategy should be applied after all file-based strategies.
    """
    
    @property
    def priority(self) -> int:
        return 999  # Lowest priority (fallback)
    
    @property
    def name(self) -> str:
        return "DefaultClientWorkload"
    
    def can_handle(self, file_desc, context: GenerationContext) -> bool:
        # This strategy is handled separately in the main loop
        return False
    
    def generate(
        self, 
        file_desc, 
        context: GenerationContext
    ) -> List[GeneratedFile]:
        # This method is not used; see generate_default_workload_if_needed
        return []
    
    @staticmethod
    def should_generate(context: GenerationContext) -> bool:
        """Check if default workload should be generated."""
        return (
            len(context.remote_services) > 0 and
            not context.has_local_workload
        )
    
    @staticmethod
    def generate_default_workload(
        context: GenerationContext
    ) -> GeneratedFile:
        """Generate default client workload."""
        from . import generators
        
        output = generators.generate_client_workload(context.remote_services)
        
        print(
            f"INFO: Generated default client workload (no local services found)",
            file=sys.stderr
        )
        
        return GeneratedFile(name=output["name"], content=output["content"])


def create_default_strategies() -> List[GenerationStrategy]:
    """Create the default list of strategies."""
    return [
        EmptyLocalWorkloadStrategy(),
        RemoteServiceStrategy(),
        LocalServiceStrategy(),
    ]
