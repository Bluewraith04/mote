from dataclasses import dataclass, field
from typing import List, Union, Optional

# ======== AST Nodes ========
# These dataclasses define the structure of parsed code.
# New language constructs (e.g., If, While, Function) can be added here.

# === Base Expression ===
@dataclass
class BinaryOp:
    op: str
    left: 'Expr'
    right: 'Expr'

@dataclass
class UnaryOp:
    op: str
    expr: 'Expr'

@dataclass
class Literal:
    value: int | float | str | bool | None

@dataclass
class Identifier:
    name: str

@dataclass
class FunctionCall:
    func: 'Expr | None'
    args: List['Expr']

@dataclass
class MemberAccess:
    obj: 'Expr | None'
    field: str

@dataclass
class IndexAccess:
    array: 'Expr | None'
    index: 'Expr'

@dataclass
class StructLiteral:
    type_name: str
    fields: List['FieldAssign']

@dataclass
class FieldAssign:
    name: str
    value: 'Expr'

@dataclass
class ArrayLiteral:
    elements: List['Expr']

Expr = Union[
    BinaryOp, UnaryOp, Literal, Identifier,
    FunctionCall, MemberAccess, IndexAccess,
    StructLiteral, ArrayLiteral
]

AssignTarget = Union[
    Identifier, MemberAccess, IndexAccess
]

# === Import ===
@dataclass
class Import:
    path: str # StringLiteral

# === Declarations ===
@dataclass
class FunctionDecl:
    name: str
    parameters: List[Identifier]
    body: 'Block'

@dataclass
class StructDecl:
    name: str
    fields: List[str]

# === Statements ===
@dataclass
class VariableDecl:
    name: str
    expr: 'Expr'

@dataclass
class Assignment: 
    target: 'AssignTarget'
    expr: 'Expr'

@dataclass
class IfStmt:
    branches: List[tuple['Expr', 'Block']]
    else_block: Optional['Block']

@dataclass
class WhileStmt:
    condition: 'Expr'
    body: 'Block'

@dataclass
class ForStmt:
    var: str
    iterable: 'Expr'
    body: 'Block'

@dataclass
class ReturnStmt:
    value: Optional['Expr']

@dataclass
class ExprStmt:
    expr: 'Expr'

@dataclass
class Block:
    statements: List['Statement']

Statement = Union[
    VariableDecl, Assignment, IfStmt, WhileStmt, ForStmt,
    ReturnStmt, ExprStmt, Block
]

# === Top-Level ===
@dataclass
class Program:
    declarations: List['DeclarationOrStatement']

DeclarationOrStatement = Union['Import', 'FunctionDecl', 'StructDecl', 'Statement']
