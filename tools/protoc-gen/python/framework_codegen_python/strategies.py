"""
Code generation strategies for different proto file scenarios.

This module implements the Strategy pattern to handle different code generation scenarios:
1. Empty local workload (pure client)
2. Remote service (dependency)
3. Local service (server implementation)
4. Default client workload (fallback)
"""

from __future__ import annotations

import abc
import sys
from typing import List, Dict, Optional, Any
from dataclasses import dataclass

from google.protobuf.compiler import plugin_pb2 as plugin


@dataclass
class GeneratedFile:
    """Represents a generated file."""
    name: str
    content: str


@dataclass
class RemoteServiceInfo:
    """Information about a remote service for proxying."""
    service_name: str
    route_keys: List[str]
    actr_type: str  # e.g., "acme+DataStreamConcurrentServer"


class GenerationContext:
    """Context for code generation, shared across all strategies."""
    
    def __init__(
        self,
        remote_file_to_actr_type: Dict[str, str],
        local_files_set: set,
        remote_services: List[RemoteServiceInfo],
    ):
        self.remote_file_to_actr_type = remote_file_to_actr_type
        self.local_files_set = local_files_set
        self.remote_services = remote_services
        self.has_local_workload = False
    
    def is_remote(self, file_desc) -> bool:
        """Check if a file is a remote dependency."""
        normalized_name = file_desc.name.replace("\\", "/")
        return normalized_name in self.remote_file_to_actr_type
    
    def is_local(self, file_desc) -> bool:
        """Check if a file is explicitly marked as local."""
        normalized_name = file_desc.name.replace("\\", "/")
        return normalized_name in self.local_files_set
    
    def has_services(self, file_desc) -> bool:
        """Check if a file has service definitions."""
        return bool(file_desc.service)


class GenerationStrategy(abc.ABC):
    """Abstract base class for code generation strategies."""
    
    @abc.abstractmethod
    def can_handle(self, file_desc, context: GenerationContext) -> bool:
        """
        Determine if this strategy can handle the given file.
        
        Args:
            file_desc: Proto file descriptor
            context: Generation context
            
        Returns:
            True if this strategy can handle the file
        """
        pass
    
    @abc.abstractmethod
    def generate(
        self, 
        file_desc, 
        context: GenerationContext
    ) -> List[GeneratedFile]:
        """
        Generate code for the given file.
        
        Args:
            file_desc: Proto file descriptor
            context: Generation context
            
        Returns:
            List of generated files
        """
        pass
    
    @property
    @abc.abstractmethod
    def priority(self) -> int:
        """
        Strategy priority (lower number = higher priority).
        
        Strategies are evaluated in priority order.
        """
        pass
    
    @property
    @abc.abstractmethod
    def name(self) -> str:
        """Strategy name for logging and debugging."""
        pass


class StrategySelector:
    """Selects the appropriate strategy for a given file."""
    
    def __init__(self, strategies: List[GenerationStrategy]):
        """
        Initialize the selector with a list of strategies.
        
        Args:
            strategies: List of available strategies
        """
        self.strategies = sorted(strategies, key=lambda s: s.priority)
    
    def select_strategy(
        self, 
        file_desc, 
        context: GenerationContext
    ) -> Optional[GenerationStrategy]:
        """
        Select the first strategy that can handle the file.
        
        Args:
            file_desc: Proto file descriptor
            context: Generation context
            
        Returns:
            Selected strategy or None if no strategy can handle the file
        """
        for strategy in self.strategies:
            if strategy.can_handle(file_desc, context):
                print(
                    f"DEBUG: Selected strategy '{strategy.name}' for file '{file_desc.name}'",
                    file=sys.stderr
                )
                return strategy
        
        print(
            f"DEBUG: No strategy found for file '{file_desc.name}'",
            file=sys.stderr
        )
        return None
