(function_item
  "fn" @context
  name: (identifier) @name) @item

(native_function_item
  "fn" @context
  name: (identifier) @name) @item

(trait_member
  "fn" @context
  name: (identifier) @name) @item

(struct_item
  "struct" @context
  name: (identifier) @name) @item

(class_item
  "class" @context
  name: (identifier) @name) @item

(enum_item
  "enum" @context
  name: (identifier) @name) @item

(trait_item
  "trait" @context
  name: (identifier) @name) @item

(type_alias
  "type" @context
  name: (identifier) @name) @item

(test_item
  (test_keyword) @context
  name: (string_literal) @name) @item
