# ======== Environment ========
# Handles values, and context specific behavior

from dataclasses import dataclass
from typing import Any, Optional


@dataclass
class Symbol:
    value: Any
    kind: Optional[str] = ''
    is_mutable: bool = True
    
    def __post_init__(self):
        self.kind = self._infer_type(self.value) if not self.kind else self.kind
        
    def _infer_type(self, value):
        if value is None: return 'null'
        if callable(value): return 'function'
        match value:
            case bool(): return 'bool'
            case int(): return 'int'
            case float(): return 'float'
            case str(): return 'string'
            case dict(): return 'struct'
            case list(): return 'array'
            case _: return 'unknown'
            
    def __str__(self) -> str:
        if self.kind == "unknown": raise ValueError("Cannot cast unknown value type into object of type 'string'.")
        return f"{self.value}"
        
    
null = Symbol(
    None,
    'null',
    False
)
    
class Environment:
    def __init__(self, parent: Optional['Environment'] = None) -> None:
        self.symbols: dict[str, Symbol] = {}
        self.parent = parent
        
    def define(self, name: str, symbol: Symbol, redefine=False) -> None:
        if name in self.symbols and not redefine:
            raise RuntimeError(f"Attempted double definition of variable '{name}'")
        self.symbols[name] = symbol
        
    def lookup(self, name: str) -> Optional['Environment']:
        if name in self.symbols:
            return self
        if self.parent:
            return self.parent.lookup(name)
        else:
            return None
        
    def has(self, name: str):
        return self.lookup(name) is not None
    
    def assign(self, name, value):
        env = self.lookup(name)
        if env:
            obj = env.symbols[name]
            if obj.is_mutable:
                obj.value = value
            else:
                raise RuntimeError(f"Attempted assignment to immutable object of kind '{obj.kind}'")
        else:
            raise RuntimeError(f"Attempted assignment to undefined variable '{name}'")
            
    def get(self, name):
        env = self.lookup(name)
        if env:
            obj = env.symbols[name]
            return obj.value
        else:
            # raise RuntimeError(f"Undefined variable '{name}'")
            return None
    
    def ref(self, name):
        env = self.lookup(name)
        if env:
            obj = env.symbols[name]
            return obj
        else:
            # raise RuntimeError(f"Undefined variable '{name}'")
            return null
    