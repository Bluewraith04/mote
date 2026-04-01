import ply.lex as lex

# Reserved keywords
reserved = {
    'import': 'IMPORT',
    'fn': 'FN',
    'struct': 'STRUCT',
    'let': 'LET',
    'if': 'IF',
    'elif': 'ELIF',
    'else': 'ELSE',
    'while': 'WHILE',
    'for': 'FOR',
    'in': 'IN',
    'return': 'RETURN',
    # 'true': 'TRUE',
    # 'false': 'FALSE',
    # 'null': 'NULL'
}

# Token list (including reserved words)
tokens = [
    'ID', 'INTEGER', 'FLOAT', 'STRING',"POWER",
    'PLUS', 'MINUS', 'TIMES', 'DIVIDE', 'MOD', 
    'EQ', 'NE', 'LT', 'GT', 'LE', 'GE',
    'AND', 'OR', 'NOT',
    'ASSIGN', 'DOT', 'COMMA', 'COLON',
    'LPAREN', 'RPAREN', 'LBRACE', 'RBRACE',
    'LBRACKET', 'RBRACKET', 'SEMICOLON',
    'TRUE', 'FALSE', 'NULL', 'INDENT', 'DEDENT', 'NEWLINE'
] + list(reserved.values())

# Operator symbols
t_POWER   = r'\*\*'
t_PLUS    = r'\+'
t_MINUS   = r'-'
t_TIMES   = r'\*'
t_DIVIDE  = r'/'
t_MOD     = r'%'
t_EQ      = r'=='
t_NE      = r'!='
t_LT      = r'<'
t_GT      = r'>'
t_LE      = r'<='
t_GE      = r'>='
t_AND     = r'&&'
t_OR      = r'\|\|'
t_NOT     = r'!'
t_ASSIGN  = r'='
t_DOT     = r'\.'
t_COMMA   = r','
t_COLON   = r':'
t_LPAREN  = r'\('
t_RPAREN  = r'\)'
t_LBRACE  = r'\{'
t_RBRACE  = r'\}'
t_LBRACKET= r'\['
t_RBRACKET= r'\]'
t_SEMICOLON = r';'

# Literal processing
def t_FLOAT(t):
    r'\d+\.\d+'
    t.value = float(t.value)
    return t

def t_INTEGER(t):
    r'\d+'
    t.value = int(t.value)
    return t

def t_STRING(t):
    r'\"([^\\\"]|\\.)*?\"'
    t.value = bytes(t.value[1:-1], 'utf-8').decode('unicode_escape')
    return t

# Boolean and Null Values
def t_TRUE(t):
    r'true'
    t.value = True
    return t

def t_FALSE(t):
    r'false'
    t.value = False
    return t

def t_NULL(t):
    r'null'
    t.value = None

# Identifier handling
def t_ID(t):
    r'[a-zA-Z_][a-zA-Z0-9_]*'
    t.type = reserved.get(t.value, 'ID')
    return t

# Comment handling
def t_LINE_COMMENT(t):
    r'//.*'
    pass

def t_BLOCK_COMMENT(t):
    r'/\*(?s:.*?)\*/'
    pass

# Ignore whitespace (except newlines)
t_ignore = ' \t\r'

# Track newlines and generate NEWLINE tokens with length of indent
def t_NEWLINE(t):
    r'\n+[ \t]*'
    t.lexer.lineno += t.value.count('\n')
    t.value = len(t.value) - t.value.rfind('\n') - 1
    return t

# Error handling
def t_error(t):
    print(f"Illegal character '{t.value[0]}' at line {t.lineno}")
    t.lexer.skip(1)

# Build the base lexer
_base_lexer = lex.lex()

class IndentationLexer:
    def __init__(self, lexer):
        self.lexer = lexer
        self.token_stream = None

    def input(self, data):
        self.lexer.input(data)
        self.token_stream = self._generate_tokens()

    def token(self):
        try:
            return next(self.token_stream)
        except StopIteration:
            return None

    def _generate_tokens(self):
        indent_stack = [0]
        pending_newlines = []
        for tok in self.lexer:
            if tok.type == 'NEWLINE':
                pending_newlines.append(tok)
                continue
            if pending_newlines:
                last_nl = pending_newlines[-1]
                pending_newlines = []
                indent = last_nl.value
                if indent > indent_stack[-1]:
                    indent_stack.append(indent)
                    last_nl.type = 'INDENT'
                    yield last_nl
                elif indent < indent_stack[-1]:
                    while indent < indent_stack[-1]:
                        indent_stack.pop()
                        import copy
                        dedent_tok = copy.copy(last_nl)
                        dedent_tok.type = 'DEDENT'
                        dedent_tok.value = indent_stack[-1]
                        yield dedent_tok
                    if indent != indent_stack[-1]:
                        print(f"IndentationError at line {last_nl.lineno}")
            yield tok

        while len(indent_stack) > 1:
            indent_stack.pop()
            import ply.lex as lex
            dedent_tok = lex.LexToken()
            dedent_tok.type = 'DEDENT'
            dedent_tok.value = 0
            dedent_tok.lineno = self.lexer.lineno
            dedent_tok.lexpos = self.lexer.lexpos
            yield dedent_tok

lexer = IndentationLexer(_base_lexer)