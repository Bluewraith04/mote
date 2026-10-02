(function_item
  "fn" @context
  name: (identifier) @name) @item

(native_function
  "fn" @context
  name: (identifier) @name) @item

(trait_method
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

(impl_item
  "impl" @context) @item

(test_declaration
  keyword: (identifier) @context
  (string) @name) @item
