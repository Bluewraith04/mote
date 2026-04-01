# from typing import Any, Optional, Callable
from .env import Symbol, null

def dprint(*args):
    print(*args)
    
types = {}
    
def dtype(obj):
    if type(obj) == Symbol:
        return obj.kind
    if callable(obj):
        return "function"
    match obj:
            case bool(): return 'bool'
            case int(): return 'int'
            case float(): return 'float'
            case str(): return 'string'
            case dict(): return 'struct'
            case list(): return 'array'
            case _: return 'unknown'
            
def dlen(obj):
    if isinstance(obj, Symbol) and isinstance(obj.value, list): obj = obj.value
    if isinstance(obj, (list, str)): return len(obj)
    raise RuntimeError("len() only supports arrays and strings")

def load_stdlib(env):
    for f in {
        "print": Symbol(dprint, "function", False),
        "len": Symbol(dlen, "function", False),
        "type": Symbol(dtype, "function", False)
    }.items(): env.define(f[0], f[1])
        
    
    