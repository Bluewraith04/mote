(line_comment) @comment
(block_comment) @comment
(doc_comment) @comment.doc

(string_literal) @string
(string_content) @string
(escape_sequence) @string.escape
(interpolation
  "${" @punctuation.special
  "}" @punctuation.special)

(integer_literal) @number
(float_literal) @number
(boolean_literal) @boolean
(null_literal) @constant.builtin

(identifier) @variable

((identifier) @constructor
  (#match? @constructor "^[A-Z]"))

(self_value) @variable.special
(self_parameter) @variable.special
(self_type) @type.builtin

(parameter name: (identifier) @variable.parameter)
(lambda_parameter (identifier) @variable.parameter)
(variadic_parameter (identifier) @variable.parameter)
(argument (identifier) @variable.parameter "=")
(generic_parameter (identifier) @type)

(field_declaration name: (identifier) @property)
(field_initializer (identifier) @property ":")
(field_pattern (identifier) @property)
(member_expression field: (identifier) @property)
(optional_member_expression field: (identifier) @property)
(tuple_index_expression index: (integer_literal) @property)

(named_type (path (identifier) @type))
(generic_type (path (identifier) @type))
(struct_item name: (identifier) @type)
(class_item name: (identifier) @type)
(enum_item name: (identifier) @enum)
(trait_item name: (identifier) @type)
(type_alias name: (identifier) @type)
(variant name: (identifier) @variant)
(struct_expression name: (identifier) @type)

((named_type (path (identifier) @type.builtin))
  (#any-of? @type.builtin "Int" "Float" "Bool" "Char" "String" "Null" "Bytes"))

(function_item name: (identifier) @function)
(native_function_item name: (identifier) @function)
(trait_member name: (identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression field: (identifier) @function))
(call_expression function: (type_apply_callee (identifier) @function))

(module_path (identifier) @namespace)
(import_name name: (identifier) @variable)
(import_item alias: (identifier) @namespace)

(attribute "@" @attribute)
(attribute (identifier) @attribute)

(test_keyword) @keyword
(send_keyword) @keyword

[
  "let"
  "var"
  "fn"
  "struct"
  "class"
  "enum"
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
  "is"
  "super"
] @keyword

(visibility) @keyword
(continue_statement) @keyword

[
  "+" "-" "*" "/" "%"
  "==" "!=" "<" "<=" ">" ">="
  "&&" "||" "&" "|" "^" "~" "!"
  "<<" ">>"
  "=" "+=" "-=" "*=" "/=" "%=" "&=" "|=" "^=" "<<=" ">>=" "??="
  ".." "..=" "..."
  "??" "?" "=>" "->"
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," ";" ":" "." "?."] @punctuation.delimiter
