; Comments
(line_comment) @comment
(block_comment) @comment
(doc_comment) @comment.doc

; Literals
(string) @string
(string_content) @string
(escape_sequence) @string.escape
(interpolation
  "${" @punctuation.special
  "}" @punctuation.special)
(char) @string
(integer) @number
(float) @number
(boolean) @boolean
(null) @constant.builtin

; Names
(identifier) @variable

; Capitalised names read as constructors unless a later pattern says otherwise
((identifier) @constructor
  (#match? @constructor "^[A-Z]"))

(self) @variable.special
(self_parameter) @variable.special
(self_type) @type.builtin

(parameter name: (identifier) @variable.parameter)
(lambda_parameter name: (identifier) @variable.parameter)
(variadic_parameter name: (identifier) @variable.parameter)
(type_parameter name: (identifier) @type)

(field_declaration name: (identifier) @property)
(variant_field name: (identifier) @property)
(field_initializer name: (identifier) @property)
(field_pattern name: (identifier) @property)
(member_expression property: (identifier) @property)
(tuple_index) @property

; Types
(named_type (identifier) @type)
(struct_item name: (identifier) @type)
(class_item name: (identifier) @type)
(enum_item name: (identifier) @enum)
(trait_item name: (identifier) @type)
(type_alias name: (identifier) @type)
(enum_variant name: (identifier) @variant)

((named_type (identifier) @type.builtin)
  (#any-of? @type.builtin "Int" "Float" "Bool" "Char" "String" "Null" "Bytes"))

; Functions
(function_item name: (identifier) @function)
(native_function name: (identifier) @function)
(trait_method name: (identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (identifier) @function))
(call_expression
  function: (identifier) @constructor
  (#match? @constructor "^[A-Z]"))
(call_expression
  function: (member_expression property: (identifier) @constructor)
  (#match? @constructor "^[A-Z]"))

; Modules and imports
(module_path (identifier) @namespace)
(import_symbol name: (identifier) @variable)
(import_symbol alias: (identifier) @variable)

; Attributes
(attribute "@" @attribute)
(attribute (identifier) @attribute)

; Tests
(test_declaration keyword: (identifier) @keyword)

; Keywords
[
  "let"
  "var"
  "fn"
  "struct"
  "class"
  "enum"
  "impl"
  "trait"
  "type"
  "import"
  "from"
  "as"
  "native"
  "scope"
  "spawn"
  "with"
  "return"
  "if"
  "else"
  "match"
  "while"
  "for"
  "in"
  "break"
  "yield"
  "try"
  "is"
] @keyword

(visibility) @keyword
(super) @keyword
(continue_statement) @keyword

; Operators
[
  "+" "-" "*" "/" "%"
  "==" "!=" "<" "<=" ">" ">="
  "&&" "||" "&" "|" "^" "~" "!"
  "<<"
  "=" "+=" "-=" "*=" "/=" "%=" "&=" "|=" "^=" "<<="
  ".." "..="
  "??" "?" "=>" "->"
] @operator

; Punctuation
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," ";" ":" "."] @punctuation.delimiter
