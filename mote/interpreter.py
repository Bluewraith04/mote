from .ast_nodes import *
from typing import Any, List, Tuple, Callable
from .env import Symbol, Environment, null
from .lexer import lexer
from .parser import parser
from .utils import *
from .stdlib import load_stdlib

class ReturnException(Exception):
    """Exception raised for return statements in function evaluation."""
    def __init__(self, value: Any):
        self.value = value


class Interpreter:
    """
    The main interpreter class responsible for evaluating the Abstract Syntax Tree (AST).
    """
    def __init__(self):
        # Initialize the global environment
        self.global_env = Environment()
        load_stdlib(self.global_env)
        self.env = self.global_env # The current active environment
        
    def execute(self, source):
        program = parser.parse(source, lexer=lexer)
        self.eval(program)
        
    def eval(self, node: Any) -> Any:
        """
        Generic evaluation method that dispatches to specific eval_ methods
        based on the AST node's class name.
        """
        # Using hasattr and getattr for dynamic method dispatch is standard and maintainable
        # as it avoids long if/elif chains.
        method_name = 'eval_' + node.__class__.__name__
        if not hasattr(self, method_name):
            raise NotImplementedError(f"Evaluation method '{method_name}' not implemented for AST node type {node.__class__.__name__}")
        return getattr(self, method_name)(node)
    
    # Expressions
    def eval_BinaryOp(self, node: BinaryOp) -> Any:
        """Evaluates a binary operation (e.g., +, -, ==)."""
        left = self.eval(node.left)
        right = self.eval(node.right)
        # Ensure that operands are raw values, not Symbols, for operations.
        # eval_Identifier already returns values, so this should be fine.

        return binary_op(node.op, left, right)

    def eval_UnaryOp(self, node: UnaryOp) -> Any:
        """Evaluates a unary operation (e.g., -, !)."""
        operand = self.eval(node.expr)
        if isinstance(operand, Symbol): operand = operand.value

        match node.op:
            case '-': 
                if not isinstance(operand, (int, float)):
                    raise RuntimeError(f"Unsupported unary operator '-' for type {type(operand).__name__}")
                return -operand
            case '!': return not bool(operand)
            case _: raise RuntimeError(f"Unknown unary operator '{node.op}'")
    
    def eval_Literal(self, node: Literal):
        """Evaluates a literal value (number, string, boolean, etc.)."""
        return node.value
    
    def eval_Identifier(self, node: Any) -> Any:
        """
        Evaluates an identifier (variable reference).
        Looks up the symbol in the environment and returns its stored value.
        """
        symbol = self.env.ref(node.name)
        if symbol is null:
            raise RuntimeError(f"Undefined variable '{node.name}'")
        return symbol # Return the actual value stored within the Symbol
    
    def eval_FunctionCall(self, node: FunctionCall) -> Any:
        """Evaluates a function call."""
        # This evaluates the function expression; this should yield a callable (from a Symbol's value)
        func_callable: Callable = unwrap(self.eval(node.func))
        
        if not callable(func_callable):
            # If the node.func was an identifier, it might have returned a non-callable symbol's value
            raise RuntimeError(f"'{node.func.name}' is not a callable function.")
        
        # This evaluates arguments, ensuring they are raw values not ast nodes
        args = [self.eval(arg) for arg in node.args]
        
        try:
            return func_callable(*args)
        except ReturnException as e:
            # This handles internal returns from interpreted functions
            return e.value
        except Exception as e:
            # This catches all other potential errors during function execution
            raise RuntimeError(f"Error during function call to '{getattr(node.func, 'name', 'anonymous_function')}': {e}")
        
    def eval_MemeberAccess(self, node: MemberAccess) -> Any:
        """Evaluates a member access on a struct"""
        obj: Symbol | dict = self.eval(node.obj)
        if isinstance(obj, Symbol): return obj.value[node.field]
        return obj[node.field]
    
    def eval_IndexAccess(self, node: IndexAccess) -> Any:
        """Evaluates an index access on an array"""
        array = self.eval(node.array)
        index = self.eval(node.index)
        if isinstance(array, Symbol): return array.value[index]
        return array[index]
    
    def eval_ArrayLiteral(self, node: ArrayLiteral) -> Any:
        """Evaluates an array literal and parses it into mote-friendly format. """
        elements = [self.eval(expr) for expr in node.elements]
        return to_symbol(*elements)
    
    def eval_StructLiteral(self, node: Any) -> Any:
        
        struct_symbol = self.env.ref(node.type_name)
        if struct_symbol is null or struct_symbol.kind != "struct_type":
            raise RuntimeError(f"Undefined struct type '{node.type_name}'")
        fields = struct_symbol.value
        instance = {'type_name': node.type_name}

        assigned_fields = set()

        for field_assign in node.fields:
            if field_assign.name not in fields:
                raise RuntimeError(f"Struct type '{node.type_name}' has no field '{field_assign.name}'")
            elif field_assign.name in assigned_fields:
                raise RuntimeError(f"Attempted multiple assignments to field '{field_assign.name}'")
            field_name, field_val = self.eval(field_assign)
            instance[field_name] = field_val
            assigned_fields.add(field_assign.name)
            
        if len(assigned_fields) != len(fields):
            missing_fields = set(fields) - assigned_fields
            raise RuntimeError(f"Missing required fields for struct type '{node.type_name}': {', '.join(missing_fields)}")
        
        return Symbol(instance)
    
    def eval_FieldAssign(self, node: FieldAssign) -> Any:
        return node.name, Symbol(self.eval(node.value))
    
    # Import
    def eval_Import(self, node: Import) -> Any:
        return
    
    # Declarations
    def eval_FunctionDecl(self, node: Any):
        """
        Evaluates a function declaration.
        Defines a callable Python function that, when called,
        creates a new environment with lexical scoping.
        """
        # Capture the environment where the function is *defined* (closure)
        outer_env_at_definition = self.env

        def interpreted_function(*args: Any) -> None:
            # Create a new environment for this specific function call
            # Its parent is the environment where the function was defined
            call_env = Environment(parent=outer_env_at_definition)

            # Check parameter count
            if len(node.parameters if node.parameters else []) != len(args):
                raise RuntimeError(f"Function '{node.name}' expected {len(node.parameters)} arguments but got {len(args)}")

            # Define parameters as symbols in the new function's environment
            if node.parameters:
                for param_node, arg_value in zip(node.parameters, args):
                    # Parameters are typically mutable local variables within the function                
                    call_env.define(param_node.name, Symbol(arg_value))

            # Save the current interpreter environment and switch to the function's environment
            previous_interpreter_env = self.env
            self.env = call_env
            try:
                self.eval(node.body) # Evaluate the function body
            except ReturnException as e:
                return e.value # Handle return statements
            finally:
                # Always restore the interpreter's environment when the function call is done
                self.env = previous_interpreter_env

        # Define the interpreted_function (the Python callable) as a Symbol in the current environment
        # Functions are typically immutable (you can't reassign the function itself)
        self.env.define(node.name, Symbol(interpreted_function, 'function', False))
    
    def eval_StructDecl(self, node: StructDecl) -> None:
        struct_symbol = Symbol(node.fields, 'struct_type', False)
        self.env.define(node.name, struct_symbol)
        
    def eval_VariableDecl(self, node: VariableDecl):
        value = self.eval(node.expr)
        self.env.define(node.name, Symbol(value, '', True))
        
    # Statements
    def eval_Assignment(self, node: Assignment):
        if not isinstance(node.target, (Identifier, IndexAccess, MemberAccess)):
            raise RuntimeError(f"Invalid assignment target: {type(node.target).__name__}")
        value = self.eval(node.expr)
        target = self.eval(node.target)
        if not isinstance(target, Symbol) and not target.is_mutable: raise TypeError("Invalid Assignment Target")
        target.value = value.value if isinstance(value, Symbol) else value
    
    def eval_IfStmt(self, node: IfStmt):        
        """Evaluates an if-else if-else statement."""
        for condition, block in node.branches:
            if self.eval(condition):
                return self.eval(block) # Execute and return if condition is true
        if node.else_block:
            return self.eval(node.else_block) # Execute else block if present
    
    def eval_WhileStmt(self, node: Any):
        """Evaluates a while loop."""
        while self.eval(node.condition):
            self.eval(node.body) # Execute loop body
    
    def eval_ForStmt(self, node: Any):
        iterable = self.eval(node.iterable)

        if isinstance(iterable, Symbol):
            iterable = iterable.value

        if not isinstance(iterable, (list, str)):
            raise RuntimeError(f"For loop expects an iterable, got {type(iterable).__name__}")
        
        previous_env = self.env

        self.env = Environment(parent=previous_env)

        try:
            for element in iterable:
                # iter_var = Symbol(node.var, "loop_variable", type_info=type(element).__name__, value=element, is_mutable=True)
                iter_var = Symbol(element, "loop_var", True)
                try:
                    self.env.define(node.var, iter_var)
                except RuntimeError:
                    self.env.assign(node.var, element)

                self.eval(node.body)
        finally:
            self.env = previous_env
            
    def eval_ReturnStmt(self, node: ReturnStmt):
        value = self.eval(node.value) if node.value else None
        raise ReturnException(value)
            
    def eval_ExprStmt(self, node: Any):
        """Evaluates an expression statement (e.g., `x + 1;`)."""
        return self.eval(node.expr)

    def eval_Block(self, node: Any):
        """Evaluates a block of statements, creating a new lexical scope."""
        previous_env = self.env
        # Create a new Environment for the block, linking its current_scope to the parent's
        self.env = Environment(parent=previous_env)
        try:
            for stmt in node.statements:
                if stmt is not None:
                    self.eval(stmt)
        finally:
            # Ensure the environment is restored even if an error occurs
            self.env = previous_env
            
    def eval_Program(self, node: Any):
        """Evaluates the top-level program (list of declarations/statements)."""
        for decl in node.declarations:
            if decl is not None:
                self.eval(decl)