# ======== Parser (Recursive Descent) ========
# Builds an AST from a list of tokens. Modular functions make grammar extension simple.

from ply import yacc
from .lexer import tokens
from .ast_nodes import *

# === Operator Precedence ===
precedence = (
    ('left', 'OR'),
    ('left', 'AND'),
    ('left', 'EQ', 'NE'),
    ('left', 'LT', 'LE', 'GT', 'GE'),
    ('left', 'PLUS', 'MINUS'),
    ('left', 'TIMES', 'DIVIDE', 'MOD'),
    ('right', 'NOT')    
)


# === Program ===
def p_program(p):
    'program : program_items'
    p[0] = Program(declarations=p[1])

def p_program_items(p):
    """
    program_items : program_items program_item
        | program_item
    """
    if len(p) == 3:
        p[0] = p[1] + [p[2]]
    else:
        p[0] = [p[1]]

def p_program_item(p):
    """
    program_item : import_stmt
                | function_decl
                | struct_decl
                | statement
                | empty_stmt
    """
    p[0] = p[1]

def p_statement(p):
    """
    statement : variable_decl
        | assignment
        | if_stmt
        | while_stmt
        | for_stmt
        | return_stmt
        | expr_stmt
        | block
    """
    p[0] = p[1]

# === Imports ===
def p_import_stmt(p):
    'import_stmt : IMPORT STRING NEWLINE'
    p[0] = Import(path=p[2])


# === Functions ===
def p_function_decl(p):
    'function_decl : FN ID LPAREN parameters_opt RPAREN COLON NEWLINE block'
    p[0] = FunctionDecl(name=p[2], parameters=p[4], body=p[8])


def p_parameters_opt(p):
    """
    parameters_opt : parameters
        | empty
    """
    p[0] = p[1] if p[1] else None

def p_parameters(p):
    """
    parameters : ID
        | parameters COMMA ID
    """
    if len(p) == 2:
        p[0] = [Identifier(name=p[1])]
    else:
        p[0] = p[1] + [Identifier(name=p[3])]

# === Structs ===
def p_struct_decl(p):
    'struct_decl : STRUCT ID COLON NEWLINE INDENT struct_fields DEDENT'
    p[0] = StructDecl(name=p[2], fields=p[5])

def p_struct_fields(p):
    """
    struct_fields : ID COMMA struct_fields
        | ID COMMA
        | empty
    """
    if len(p) == 4:
        p[0] = [p[1]] + p[3]
    elif len(p) == 3:
        p[0] = [p[1]]
    else:
        p[0] = []


# === Variable ===
def p_variable_decl(p):
    'variable_decl : LET ID ASSIGN expr NEWLINE'
    p[0] = VariableDecl(name=p[2], expr=p[4])

# === Assignment ===
def p_assignment(p):
    'assignment : assign_target ASSIGN expr NEWLINE'
    p[0] = Assignment(target=p[1], expr=p[3])

def p_assign_target_id(p):
    'assign_target : ID'
    p[0] = Identifier(name=p[1])

def p_assign_target_member(p):
    'assign_target : assign_target DOT ID'
    p[0] = MemberAccess(obj=p[1], field=p[3])

def p_assign_target_index(p):
    'assign_target : assign_target LBRACKET expr RBRACKET'
    p[0] = IndexAccess(array=p[1], index=p[3])

# === Statements ===

def p_if_stmt(p):
    """
    if_stmt : IF expr COLON NEWLINE block
            | IF expr COLON NEWLINE block else_block
            | IF expr COLON NEWLINE block elif_blocks
            | IF expr COLON NEWLINE block elif_blocks else_block
    """
    if len(p) == 6:
        p[0] = IfStmt(branches=[(p[2], p[5])], else_block=None)
    elif len(p) == 7:
        if isinstance(p[6], list): # elif_blocks returns list
            p[0] = IfStmt(branches=[(p[2], p[5])] + p[6], else_block=None)
        else: # else_block returns just the block AST
            p[0] = IfStmt(branches=[(p[2], p[5])], else_block=p[6])
    else: # len 8, has both
        p[0] = IfStmt(branches=[(p[2], p[5])] + p[6], else_block=p[7])

def p_elif_blocks(p):
    """
    elif_blocks : ELIF expr COLON NEWLINE block
                | elif_blocks ELIF expr COLON NEWLINE block
    """
    if len(p) == 6:
        p[0] = [(p[2], p[5])]
    else:
        p[0] = p[1] + [(p[3], p[6])]

def p_else_block(p):
    """
    else_block : ELSE COLON NEWLINE block
    """
    p[0] = p[4]

def p_while_stmt(p):
    'while_stmt : WHILE expr COLON NEWLINE block'
    p[0] = WhileStmt(condition=p[2], body=p[5])

def p_for_stmt(p):
    'for_stmt : FOR ID IN expr COLON NEWLINE block'
    p[0] = ForStmt(var=p[2], iterable=p[4], body=p[7])

def p_return_stmt(p):
    """
    return_stmt : RETURN expr NEWLINE
        | RETURN NEWLINE
    """
    p[0] = ReturnStmt(value=p[2] if len(p) == 4 else None)

def p_expr_stmt(p):
    'expr_stmt : expr NEWLINE'
    p[0] = ExprStmt(expr=p[1])

def p_empty_stmt(p):
    'empty_stmt : NEWLINE'
    pass

def p_block(p):
    'block : INDENT statement_list DEDENT'
    p[0] = Block(statements=p[2])

def p_statement_list(p):
    """
    statement_list : statement_list statement
        | empty
    """
    if len(p) == 3:
        p[0] = p[1] + [p[2]]
    else:
        p[0] = []

# === Expressions ===
# TODO: Add precedence rules
def p_expr_identifier(p):
    'primary_expr : ID'
    p[0] = Identifier(name=p[1])

def p_primary_expr(p):
    """
    primary_expr : literal
        | LPAREN expr RPAREN
        | struct_literal
        | array_literal
    """
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = p[2]

def p_postfix_expr(p):
    'postfix_expr : primary_expr postfix_chain'
    expr = p[1]
    for postfix in p[2]:
        if isinstance(postfix, FunctionCall):
            postfix.func = expr
            expr = postfix
        elif isinstance(postfix, IndexAccess):
            postfix.array = expr
            expr = postfix
        elif isinstance(postfix, MemberAccess):
            postfix.obj = expr
            expr = postfix
    p[0] = expr

def p_postfix_chain(p):
    """
    postfix_chain : postfix postfix_chain
        | empty
    """
    if len(p) == 3:
        p[0] = [p[1]] + p[2]
    else:
        p[0] = []
    
    
def p_postfix(p):
    """
    postfix : LPAREN args_opt RPAREN
        | DOT ID
        | LBRACKET expr RBRACKET
    """
    match p[1]:
        case '(':
            p[0] = FunctionCall(func=None, args=p[2])
        case '[':
            p[0] = IndexAccess(array=None, index=p[2])
        case '.':
            p[0] = MemberAccess(obj=None, field=p[2])

def p_unary_op(p):
    """
    unary_expr : NOT unary_expr
        | MINUS unary_expr
        | postfix_expr
    """
    if len(p) == 2:
        p[0] = p[1]
    else:
        # p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])
        p[0] = UnaryOp(op=p[1], expr=p[2])

def p_exponential_expr(p):
    """
    exponential_expr : unary_expr
        | unary_expr POWER exponential_expr
    """
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])

def p_multiplicative_expr(p):
    """
    multiplicative_expr : exponential_expr
        | multiplicative_expr TIMES exponential_expr
        | multiplicative_expr DIVIDE exponential_expr
        | multiplicative_expr MOD exponential_expr
    """
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])

def p_additive_expr(p):
    """
    additive_expr : multiplicative_expr
        | additive_expr PLUS multiplicative_expr
        | additive_expr MINUS multiplicative_expr
    """
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])

def p_relational_expr(p):
    '''relational_expr : additive_expr
                       | relational_expr LT additive_expr
                       | relational_expr GT additive_expr
                       | relational_expr LE additive_expr
                       | relational_expr GE additive_expr'''
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])

def p_equality_expr(p):
    '''equality_expr : relational_expr
                     | equality_expr EQ relational_expr
                     | equality_expr NE relational_expr'''
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])

def p_logical_and_expr(p):
    '''logical_and_expr : equality_expr
                        | logical_and_expr AND equality_expr'''
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])

def p_logical_or_expr(p):
    '''logical_or_expr : logical_and_expr
                       | logical_or_expr OR logical_and_expr'''
    if len(p) == 2:
        p[0] = p[1]
    else:
        p[0] = BinaryOp(op=p[2], left=p[1], right=p[3])

# The actual expression rule
def p_expr(p):
    '''expr : logical_or_expr'''
    p[0] = p[1]

def p_expr_literal(p):
    """
    literal : INTEGER
        | FLOAT
        | STRING
        | TRUE
        | FALSE
        | NULL
    """
    p[0] = Literal(value=p[1])

def p_args_opt(p):
    """
    args_opt : args
        | empty
    """
    p[0] = p[1] if p[1] is not None else []

def p_args(p):
    """
    args : expr
        | args COMMA expr
    """
    if len(p) == 2:
        p[0] = [p[1]]
    else:
        p[0] = p[1] + [p[3]]

def p_struct_literal(p):
    'struct_literal : ID LBRACE field_assignments_opt RBRACE'
    p[0] = StructLiteral(type_name=p[1], fields=p[3])

def p_field_assignments_opt(p):
    """
    field_assignments_opt : field_assignments
        | empty
    """
    p[0] = p[1] if p[1] else []

def p_field_assignments(p):
    """
    field_assignments : field_assign
        | field_assignments COMMA field_assign
    """
    if len(p) == 2:
        p[0] = [p[1]]
    else:
        p[0] = p[1] + [p[3]]

def p_field_assign(p):
    'field_assign : ID COLON expr'
    p[0] = FieldAssign(name=p[1], value=p[3])

def p_array_literal(p):
    'array_literal : LBRACKET expr_list_opt RBRACKET'
    p[0] = ArrayLiteral(elements=p[2])

def p_expr_list_opt(p):
    """
    expr_list_opt : expr_list
        | empty
    """
    p[0] = p[1] if p[1] else []

def p_expr_list(p):
    """
    expr_list : expr
        | expr_list COMMA expr
    """
    if len(p) == 2:
        p[0] = [p[1]]
    else:
        p[0] = p[1] + [p[3]]
        
# TODO Implement:
# - Struct Literals (Done)
# - Array Literals (Done)
# - Function calls (Done)
# - Member Access (Done)
# - Index access (Done)


# === Miscellaneous ===
def p_empty(p):
    'empty : '
    pass

def p_error(p):
    if p:
        print(f"Syntax error at token '{p.value}', line {p.lineno}")
    else:
        print("Syntax error at EOF")


parser = yacc.yacc()