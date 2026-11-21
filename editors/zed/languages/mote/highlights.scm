(line_comment) @comment
(block_comment) @comment
(doc_comment) @comment.doc

(string_literal "\"" @string)
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

((identifier) @variable.special
  (#eq? @variable.special "_"))

((identifier) @constructor
  (#match? @constructor "^[A-Z]"))

((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))

(self_value) @variable.special
(self_parameter "self" @variable.special)
(self_type) @type.builtin

(const_decl name: (identifier) @constant)
(parameter name: (identifier) @variable.parameter)
(lambda_parameter (identifier) @variable.parameter)
(variadic_parameter (identifier) @variable.parameter)
(argument (identifier) @variable.parameter "=")
(generic_parameter (identifier) @type)
(generic_parameter "const" (identifier) @constant)

(field_declaration name: (identifier) @property)
(field_initializer (identifier) @property ":")
(field_initializer . (identifier) @property .)
(field_pattern (identifier) @property)
(member_expression field: (identifier) @property)
(optional_member_expression field: (identifier) @property)
(tuple_index_expression index: (integer_literal) @property)

(named_type (path (identifier) @namespace "."))
(generic_type (path (identifier) @namespace "."))
(named_type (path (identifier) @type .))
(generic_type (path (identifier) @type .))
(struct_item name: (identifier) @type)
(class_item name: (identifier) @type)
(enum_item name: (identifier) @enum)
(trait_item name: (identifier) @type)
(type_alias name: (identifier) @type)
(variant name: (identifier) @variant)
(struct_expression name: (identifier) @type)
(struct_expression name: (member_expression field: (identifier) @type))
((type_apply_callee (identifier) @type)
  (#match? @type "^[A-Z]"))

((named_type (path (identifier) @type.builtin .))
  (#any-of? @type.builtin "Int" "Int8" "Int16" "Int32" "Int64" "UInt8" "UInt16" "UInt32" "UInt64" "Float" "Float32" "Float64" "Bool" "String" "Null" "Any" "Bytes" "StrView" "Error" "ErrorKind" "PortEnd" "AtomicInt" "AtomicBool"))

((generic_type (path (identifier) @type.builtin .))
  (#any-of? @type.builtin "List" "Array" "Map" "Set" "Range" "Stream" "Option" "Result" "Pointer" "Task" "Sender" "Receiver" "Mutex" "Once" "Span" "MutSpan" "Buffer"))

((type_apply_callee (identifier) @type.builtin)
  (#any-of? @type.builtin "List" "Array" "Map" "Set" "Range" "Stream" "Option" "Result" "Pointer" "Task" "Sender" "Receiver" "Mutex" "Once" "Span" "MutSpan" "Buffer"))

(path_pattern (identifier) @type "." (identifier) @variant .)
(tuple_struct_pattern (path (identifier) @constructor .))
(tuple_struct_pattern (path (identifier) @type "." (identifier) @variant .))
(struct_pattern (path (identifier) @type .))
(struct_pattern (path (identifier) @type "." (identifier) @variant .))

(function_item name: (identifier) @function)
(struct_item (function_item name: (identifier) @function.method))
(class_item (function_item name: (identifier) @function.method))
(enum_item (function_item name: (identifier) @function.method))
(extend_item (function_item name: (identifier) @function.method))
(extern_fn name: (identifier) @function)
(trait_member name: (identifier) @function.method)
(call_expression function: (identifier) @function)
(call_expression function: (type_apply_callee (identifier) @function))
(call_expression function: (member_expression field: (identifier) @function.method))
(call_expression function: (optional_member_expression field: (identifier) @function.method))
(call_expression function: (type_apply_callee (member_expression field: (identifier) @function.method)))
(call_expression function: (type_apply_expression field: (identifier) @function.method))

((call_expression function: (identifier) @constructor)
  (#match? @constructor "^[A-Z]"))
((call_expression function: (member_expression field: (identifier) @constructor))
  (#match? @constructor "^[A-Z]"))
((call_expression function: (type_apply_callee (identifier) @constructor))
  (#match? @constructor "^[A-Z]"))

((call_expression function: (identifier) @function.builtin)
  (#any-of? @function.builtin "print" "println" "eprint" "eprintln" "typeof"))

(builtin_expression "@" @function.builtin (identifier) @function.builtin)
(builtin_type "@" @function.builtin (identifier) @function.builtin)

(module_path (identifier) @namespace)
(import_item alias: (identifier) @namespace)
(import_name name: (identifier) @variable)
((import_name name: (identifier) @type)
  (#match? @type "^[A-Z]"))
((import_name name: (identifier) @constant)
  (#match? @constant "^[A-Z][A-Z0-9_]+$"))
(import_name alias: (identifier) @variable)
((import_name alias: (identifier) @type)
  (#match? @type "^[A-Z]"))
(import_item "*" @operator)

(attribute "@" @attribute (identifier) @attribute)
(attribute_argument key: (path (identifier) @property))

(test_keyword) @keyword
(send_keyword) @keyword.modifier
(visibility) @keyword.modifier

[
  "let"
  "var"
  "const"
  "fn"
  "struct"
  "class"
  "enum"
  "trait"
  "type"
  "extern"
  "extend"
  "comptime"
  "super"
] @keyword

[
  "import"
  "from"
  "as"
] @keyword.import

[
  "if"
  "else"
  "match"
  "while"
  "for"
  "in"
  "return"
  "break"
  "yield"
  "scope"
  "spawn"
  "with"
] @keyword.control

(continue_statement) @keyword.control

["is"] @keyword.operator

[
  "+" "-" "*" "/" "%"
  "==" "!=" "<" "<=" ">" ">="
  "&&" "||" "&" "|" "^" "~" "!"
  "<<" ">>"
  "=" "+=" "-=" "*=" "/=" "%=" "&=" "|=" "^=" "<<=" ">>=" "??="
  ".." "..=" "..."
  "??" "?" "=>" "->"
] @operator

(at_pattern "@" @operator)

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
(generics "<" @punctuation.bracket ">" @punctuation.bracket)
(type_arguments "<" @punctuation.bracket ">" @punctuation.bracket)
(lambda_expression "|" @punctuation.bracket)
(lambda_expression "||" @punctuation.bracket)
["," ";" ":" "." "?."] @punctuation.delimiter
