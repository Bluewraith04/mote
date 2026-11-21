/// Tree-sitter grammar for Mote, written from `syntax/Syntax.md`.
/// Newlines are whitespace except that a call's `(` must be on the line of its callee (the scanner).
/// Nested block comments, `test` and `Send` are read by `src/scanner.c`.

const PREC = {
  coalesce: 1,
  or: 2,
  and: 3,
  bit_or: 4,
  bit_xor: 5,
  bit_and: 6,
  equality: 7,
  comparison: 8,
  is: 9,
  shift: 10,
  range: 11,
  additive: 12,
  multiplicative: 13,
  cast: 14,
  unary: 15,
  postfix: 16,
};

const commaSep = (rule) => optional(commaSep1(rule));
const commaSep1 = (rule) => seq(rule, repeat(seq(',', rule)), optional(','));

module.exports = grammar({
  name: 'mote',

  word: ($) => $.identifier,

  externals: ($) => [$.block_comment, $._call_paren, $._send_word, $._test_word, $._string_content, $._error_sentinel],

  extras: ($) => [/\s/, $.doc_comment, $.line_comment, $.block_comment],

  supertypes: ($) => [$._expression, $._type, $._pattern, $._statement],

  conflicts: ($) => [
    [$._primary_pattern, $._range_operand],
    [$.tuple_type, $.function_type],
    [$._expression, $.struct_expression, $.type_apply_callee],
    [$._expression, $.struct_expression],
    [$.module_path, $.import_name],
    [$.module_path],
    [$.attribute, $._builtin_arguments],
    [$.attribute_argument, $._expression],
    [$.path, $._arg_name],
    [$.path, $._expression],
    [$.path, $._expression, $.struct_expression, $.type_apply_callee],
    [$.builtin_type, $.builtin_expression],
    [$.tuple_type, $.tuple_expression],
  ],

  rules: {
    source_file: ($) => repeat(choice($._top_level, ';')),

    _top_level: ($) => seq(repeat($.attribute), choice($._item, $._statement)),

    // ---------------------------------------------------------------- tokens

    identifier: () => /[a-zA-Z_][a-zA-Z0-9_]*/,

    integer_literal: () =>
      token(
        choice(
          /0x[0-9a-fA-F_]+(_?[ui](8|16|32|64))?/,
          /0b[01_]+(_?[ui](8|16|32|64))?/,
          /0o[0-7_]+(_?[ui](8|16|32|64))?/,
          /[0-9][0-9_]*(_?[ui](8|16|32|64))?/,
        ),
      ),

    float_literal: () =>
      token(
        choice(
          /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9_]+)?(_?f(32|64))?/,
          /[0-9][0-9_]*[eE][+-]?[0-9_]+(_?f(32|64))?/,
        ),
      ),

    boolean_literal: () => choice('true', 'false'),
    null_literal: () => 'null',

    string_literal: ($) =>
      seq(
        '"',
        repeat(choice(alias($._string_content, $.string_content), $.escape_sequence, $.interpolation)),
        token.immediate('"'),
      ),

    escape_sequence: () => token.immediate(/\\(u\{[0-9a-fA-F_]+\}|[\s\S])/),
    interpolation: ($) => seq(token.immediate('${'), $._expression, '}'),

    doc_comment: () => token(prec(2, /\/\/\/([^\/\n][^\n]*)?/)),
    line_comment: () => token(prec(1, choice(/\/\/([^\/\n][^\n]*)?/, /\/\/\/\/[^\n]*/))),

    // ----------------------------------------------------------------- names

    _member_name: ($) => choice($.identifier, alias('from', $.identifier)),

    path: ($) => prec.right(seq(choice($.identifier, $.self_type), repeat(seq('.', $.identifier)))),

    module_path: ($) =>
      seq(
        repeat(choice('.', '..')),
        repeat(seq('super', '.')),
        $.identifier,
        repeat(seq('.', $.identifier)),
      ),

    // ------------------------------------------------------------ attributes

    attribute: ($) => prec.dynamic(5, prec.right(seq('@', $.identifier, optional(seq(choice('(', alias($._call_paren, '(')), commaSep($.attribute_argument), ')'))))),

    attribute_argument: ($) =>
      prec.dynamic(5, seq(optional(seq(field('key', $.path), '=')), field('value', choice($.path, $.string_literal, $.integer_literal, $.float_literal, $.boolean_literal)))),

    // ----------------------------------------------------------------- items

    _item: ($) =>
      choice(
        $.extern_block,
        $.extend_item,
        $.public_const,
        $.test_item,
        $.struct_item,
        $.class_item,
        $.enum_item,
        $.trait_item,
        $.type_alias,
        $.import_item,
        $.public_binding,
      ),

    visibility: () => 'pub',

    public_binding: ($) => seq($.visibility, $.binding),
    public_const: ($) => seq($.visibility, $.const_decl),

    function_item: ($) =>
      seq(
        optional($.visibility),
        'fn',
        field('name', $._member_name),
        optional($.generics),
        $.parameters,
        optional(seq('->', field('return_type', $._type))),
        field('body', $.block),
      ),

    extern_block: ($) =>
      seq(optional($.visibility), 'extern', field('library', $.string_literal), '{', repeat($.extern_fn), '}'),

    extern_fn: ($) =>
      seq(
        repeat($.attribute),
        optional($.visibility),
        'fn',
        field('name', $._member_name),
        $.parameters,
        optional(seq('->', field('return_type', $._type))),
      ),

    extend_item: ($) =>
      seq(
        'extend',
        optional($.generics),
        field('target', $._type),
        optional($.trait_list),
        '{',
        repeat($.function_item),
        '}',
      ),

    const_decl: ($) =>
      seq('const', field('name', $.identifier), optional(seq(':', field('type', $._type))), '=', field('value', $._expression)),

    test_item: ($) => seq(alias($._test_word, $.test_keyword), field('name', $.string_literal), field('body', $.block)),

    struct_item: ($) => seq(optional($.visibility), 'struct', field('name', $.identifier), optional($.generics), optional($.trait_list), $._record_body),

    class_item: ($) => seq(optional($.visibility), 'class', field('name', $.identifier), optional($.generics), optional($.trait_list), $._record_body),

    _record_body: ($) => seq('{', repeat(choice($.field_declaration, $.function_item)), '}'),

    field_declaration: ($) =>
      seq(optional($.visibility), optional(choice('var', 'let')), field('name', $.identifier), ':', field('type', $._type), optional(seq('=', field('value', $._expression)))),

    enum_item: ($) =>
      seq(optional($.visibility), 'enum', field('name', $.identifier), optional($.generics), optional($.trait_list), '{', repeat(choice(seq($.variant, optional(',')), $.function_item)), '}'),

    variant: ($) =>
      seq(
        repeat($.attribute),
        field('name', $.identifier),
        optional(
          choice(
            seq('(', commaSep($._type), ')'),
            seq('{', repeat(seq($.field_declaration, optional(','))), '}'),
            seq('=', $._expression),
          ),
        ),
      ),

    trait_item: ($) => seq(optional($.visibility), 'trait', field('name', $.identifier), optional($.generics), '{', repeat($.trait_member), '}'),

    trait_member: ($) =>
      seq('fn', field('name', $._member_name), optional($.generics), $.parameters, optional(seq('->', field('return_type', $._type))), optional(field('body', $.block))),

    type_alias: ($) => seq(optional($.visibility), 'type', field('name', $.identifier), optional($.generics), '=', field('value', $._type)),

    import_item: ($) =>
      seq(
        optional($.visibility),
        'import',
        choice(
          seq(
            field('path', $.module_path),
            choice(
              optional(seq('as', field('alias', $.identifier))),
              seq('.', choice('*', seq('{', choice(commaSep1($.import_name), '*'), '}'))),
            ),
          ),
          seq(choice($.import_name, '*'), 'from', field('path', $.module_path)),
          seq('{', choice(commaSep1($.import_name), '*'), '}', 'from', field('path', $.module_path)),
        ),
      ),

    import_name: ($) => seq(field('name', $.identifier), optional(seq('as', field('alias', $.identifier)))),

    generics: ($) => seq('<', commaSep1($.generic_parameter), '>'),

    generic_parameter: ($) =>
      choice(
        seq('const', $.identifier, ':', $._type),
        seq($.identifier, optional(seq(':', $._type, repeat(seq('+', $._type))))),
      ),

    trait_list: ($) => seq(':', '(', commaSep1($._type), ')'),

    parameters: ($) => seq('(', commaSep($._parameter), ')'),

    _parameter: ($) => choice($.self_parameter, $.variadic_parameter, $.parameter),

    self_parameter: () => seq(optional('var'), 'self'),

    variadic_parameter: ($) => seq('...', $.identifier, optional(seq(':', $._type))),

    parameter: ($) => seq(optional('var'), field('name', $.identifier), optional(seq(':', field('type', $._type))), optional(seq('=', field('default', $._expression)))),

    // ------------------------------------------------------------ statements

    block: ($) => prec(1, seq('{', repeat(choice($._statement, ';')), '}')),

    _statement: ($) =>
      choice(
        $.binding,
        $.const_decl,
        $.assignment,
        $.while_statement,
        $.for_statement,
        $.comptime_for_statement,
        $.scope_statement,
        $.with_statement,
        $.return_statement,
        $.break_statement,
        $.continue_statement,
        $.yield_statement,
        $.function_item,
        $._expression,
      ),

    binding: ($) => seq(choice('let', 'var'), field('pattern', $._binding_pattern), optional(seq(':', field('type', $._type))), '=', field('value', $._expression)),

    assignment: ($) =>
      prec.right(
        seq(field('left', $._expression), field('operator', choice('=', '+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '<<=', '>>=', '??=')), field('right', $._expression)),
      ),

    _condition: ($) => choice($.let_condition, $._expression),

    let_condition: ($) => seq('let', field('pattern', $._pattern), '=', field('value', $._expression)),

    while_statement: ($) => seq('while', field('condition', $._condition), field('body', $.block)),

    for_statement: ($) => seq('for', field('pattern', $._binding_pattern), 'in', field('iterable', $._expression), field('body', $.block)),

    comptime_for_statement: ($) => seq('comptime', 'for', field('pattern', $._binding_pattern), 'in', field('iterable', $._expression), field('body', $.block)),

    comptime_block: ($) => seq('comptime', $.block),

    scope_statement: ($) => seq('scope', $.block),

    with_statement: ($) => seq('with', field('name', $.identifier), '=', field('value', $._expression), field('body', $.block)),

    return_statement: ($) => prec.right(seq('return', optional($._expression))),
    break_statement: ($) => prec.right(seq('break', optional($._expression))),
    continue_statement: () => 'continue',
    yield_statement: ($) => prec.right(seq('yield', $._expression)),

    // -------------------------------------------------------------- patterns

    // A pattern in a match arm or a condition may carry a type (`x: Int`); one in a binding or a field may not.
    _pattern: ($) => choice($._pattern_alternative, $.or_pattern),

    _pattern_alternative: ($) => choice($.type_pattern, $._primary_pattern, $.tuple_pattern, $.tuple_struct_pattern),

    _binding_pattern: ($) => choice($._binding_alternative, alias($._binding_or_pattern, $.or_pattern)),

    _binding_alternative: ($) => choice($._primary_pattern, alias($._binding_tuple_pattern, $.tuple_pattern), alias($._binding_tuple_struct_pattern, $.tuple_struct_pattern)),

    _primary_pattern: ($) => choice($.binding_pattern, $.at_pattern, $.literal_pattern, $.range_pattern, $.path_pattern, $.struct_pattern),

    type_pattern: ($) => seq($.identifier, ':', $._type),
    binding_pattern: ($) => $.identifier,
    at_pattern: ($) => seq($.identifier, '@', choice($._range_operand, $.range_pattern, $.struct_pattern, $.tuple_pattern, $.tuple_struct_pattern)),
    _range_operand: ($) => choice($.binding_pattern, $.literal_pattern, $.path_pattern),
    literal_pattern: ($) => choice($.integer_literal, seq('-', $.integer_literal), $.string_literal, $.boolean_literal, $.null_literal),
    range_pattern: ($) => prec.left(1, seq($._range_operand, choice('..', '..='), $._range_operand)),
    or_pattern: ($) => seq($._pattern_alternative, repeat1(seq('|', $._pattern_alternative))),
    _binding_or_pattern: ($) => seq($._binding_alternative, repeat1(seq('|', $._binding_alternative))),
    tuple_pattern: ($) => seq('(', commaSep($._pattern), ')'),
    _binding_tuple_pattern: ($) => seq('(', commaSep($._binding_pattern), ')'),
    path_pattern: ($) => seq($.identifier, repeat1(seq('.', $.identifier))),
    tuple_struct_pattern: ($) => seq($.path, '(', commaSep($._pattern), ')'),
    _binding_tuple_struct_pattern: ($) => seq($.path, '(', commaSep($._binding_pattern), ')'),
    struct_pattern: ($) => seq($.path, '{', repeat(seq($.field_pattern, optional(','))), optional('..'), '}'),
    field_pattern: ($) => seq($.identifier, optional(seq(':', $._binding_pattern))),

    // ----------------------------------------------------------------- types

    _type_atom: ($) => choice($.named_type, $.generic_type, $.builtin_type, $.nullable_type, $.tuple_type, $.array_type, $.function_type, $.parenthesized_type),

    _type: ($) => choice($._type_atom, $.union_type),

    named_type: ($) => prec(-1, $.path),

    generic_type: ($) => prec(1, seq($.path, $.type_arguments)),

    builtin_type: ($) => seq('@', $.identifier, optional($.type_arguments), alias($._builtin_arguments, $.arguments)),

    type_arguments: ($) => seq('<', commaSep1(choice($._type, $._const_type_arg)), '>'),

    _const_type_arg: ($) =>
      choice(
        $.integer_literal,
        $.float_literal,
        $.boolean_literal,
        $.string_literal,
        $.null_literal,
        alias($._const_unary, $.unary_expression),
        alias($._const_binary, $.binary_expression),
      ),

    _const_atom: ($) =>
      choice(
        $.identifier,
        $.integer_literal,
        $.float_literal,
        $.boolean_literal,
        $.string_literal,
        $.null_literal,
        alias($._const_parens, $.parenthesized_expression),
      ),

    _const_parens: ($) => seq('(', $._const_binary, ')'),
    _const_unary: ($) => prec(PREC.unary, seq(choice('-', '!', '~'), $._const_atom)),
    _const_binary: ($) =>
      choice(
        prec.left(PREC.additive, seq(choice($._const_atom, $._const_binary), choice('+', '-'), choice($._const_atom, $._const_binary))),
        prec.left(PREC.multiplicative, seq(choice($._const_atom, $._const_binary), choice('*', '/', '%'), choice($._const_atom, $._const_binary))),
      ),

    nullable_type: ($) => prec.left(2, seq($._type_atom, choice('?', '??'))),

    tuple_type: ($) => choice(seq('(', ')'), seq('(', $._type, ',', ')'), seq('(', $._type, repeat1(seq(',', $._type)), optional(','), ')')),

    parenthesized_type: ($) => seq('(', $._type, ')'),

    function_type: ($) => seq(optional(alias($._send_word, $.send_keyword)), '(', commaSep(seq(optional('var'), $._type)), ')', '->', $._type),

    array_type: ($) => seq('[', $._type, optional(seq(';', $._expression)), ']'),

    union_type: ($) => prec(1, seq($._type_atom, repeat1(seq('|', $._type_atom)))),

    self_type: () => 'Self',

    // ----------------------------------------------------------- expressions

    _expression: ($) =>
      choice(
        $.integer_literal,
        $.float_literal,
        $.string_literal,
        $.boolean_literal,
        $.null_literal,
        $.identifier,
        $.self_value,
        $.self_type,
        $.parenthesized_expression,
        $.tuple_expression,
        $.list_expression,
        $.array_repeat_expression,
        $.map_expression,
        $.struct_expression,
        $.lambda_expression,
        $.call_expression,
        $.builtin_expression,
        $.member_expression,
        $.tuple_index_expression,
        $.optional_member_expression,
        $.index_expression,
        $.slice_expression,
        $.type_apply_expression,
        $.unwrap_expression,
        $.propagate_expression,
        $.unary_expression,
        $.binary_expression,
        $.type_test_expression,
        $.cast_expression,
        $.range_expression,
        $.spawn_expression,
        $.if_expression,
        $.comptime_if_expression,
        $.comptime_block,
        $.match_expression,
      ),

    self_value: () => 'self',

    parenthesized_expression: ($) => seq('(', $._expression, ')'),

    tuple_expression: ($) => choice(seq('(', ')'), seq('(', $._expression, ',', commaSep($._expression), ')')),

    list_expression: ($) => seq('[', commaSep($._expression), ']'),

    array_repeat_expression: ($) => seq('[', $._expression, ';', $._expression, ']'),

    map_expression: ($) => seq('{', commaSep(seq($._expression, ':', $._expression)), '}'),

    struct_expression: ($) => seq(field('name', choice($.identifier, $.member_expression)), optional($.type_arguments), '{', repeat(seq($.field_initializer, optional(','))), '}'),

    field_initializer: ($) => choice(seq($.identifier, ':', $._expression), $.identifier, seq('..', $._expression)),

    lambda_expression: ($) =>
      prec.right(
        -1,
        seq(
          choice(seq('|', commaSep($.lambda_parameter), '|'), '||'),
          optional(seq('->', alias($._type_atom, $.return_type))),
          choice($.block, $._expression, $.assignment),
        ),
      ),

    lambda_parameter: ($) => seq(optional('var'), $.identifier, optional(seq(':', alias($._type_atom, $.parameter_type)))),

    arguments: ($) => seq(alias($._call_paren, '('), commaSep($.argument), ')'),

    _arg_name: ($) => $.identifier,

    argument: ($) => choice(seq($._arg_name, '=', $._expression), $._expression),

    call_expression: ($) => prec(PREC.postfix, seq(field('function', choice($._expression, $.type_apply_callee)), $.arguments)),

    _builtin_arguments: ($) => seq(alias($._call_paren, '('), commaSep(choice($.argument, $._non_ident_type)), ')'),

    _non_ident_type: ($) =>
      choice($.generic_type, $.builtin_type, $.nullable_type, $.tuple_type, $.array_type, $.function_type, $.union_type),

    builtin_expression: ($) => seq('@', $.identifier, optional($.type_arguments), alias($._builtin_arguments, $.arguments)),

    type_apply_callee: ($) => seq(choice($.identifier, $.member_expression), $.type_arguments),

    type_apply_expression: ($) => prec.dynamic(1, prec(PREC.postfix, seq(field('object', $.type_apply_callee), '.', field('field', $._member_name)))),

    member_expression: ($) => prec(PREC.postfix, seq(field('object', $._expression), '.', field('field', $._member_name))),

    tuple_index_expression: ($) => prec(PREC.postfix, seq(field('object', $._expression), '.', field('index', $.integer_literal))),

    optional_member_expression: ($) => prec(PREC.postfix, seq(field('object', $._expression), '?.', field('field', $._member_name))),

    index_expression: ($) => prec(PREC.postfix, seq(field('object', $._expression), '[', field('index', $._expression), ']')),

    slice_expression: ($) =>
      prec(
        PREC.postfix,
        seq(field('object', $._expression), '[', optional($._expression), choice('..', '..='), optional($._expression), optional(seq('..', $._expression)), ']'),
      ),

    unwrap_expression: ($) => prec(PREC.postfix, seq($._expression, '!')),
    propagate_expression: ($) => prec(PREC.postfix, seq($._expression, '?')),

    unary_expression: ($) => prec(PREC.unary, seq(choice('-', '!', '~'), $._expression)),

    binary_expression: ($) => {
      const table = [
        [PREC.coalesce, '??', 'right'],
        [PREC.or, '||', 'left'],
        [PREC.and, '&&', 'left'],
        [PREC.bit_or, '|', 'left'],
        [PREC.bit_xor, '^', 'left'],
        [PREC.bit_and, '&', 'left'],
        [PREC.equality, choice('==', '!='), 'left'],
        [PREC.comparison, choice('<', '<=', '>', '>='), 'left'],
        [PREC.shift, choice('<<', '>>'), 'left'],
        [PREC.additive, choice('+', '-'), 'left'],
        [PREC.multiplicative, choice('*', '/', '%'), 'left'],
      ];
      return choice(
        ...table.map(([precedence, operator, side]) =>
          (side === 'right' ? prec.right : prec.left)(precedence, seq(field('left', $._expression), field('operator', operator), field('right', $._expression))),
        ),
      );
    },

    type_test_expression: ($) => prec.left(PREC.is, seq(field('value', $._expression), 'is', field('type', $._type))),

    cast_expression: ($) => prec.left(PREC.cast, seq(field('value', $._expression), 'as', field('type', $._type))),

    range_expression: ($) => prec.left(PREC.range, seq(field('start', $._expression), choice('..', '..='), field('end', $._expression))),

    spawn_expression: ($) => prec.right(seq('spawn', choice($.block, $._expression))),

    if_expression: ($) => seq('if', field('condition', $._condition), field('then', $.block), optional(seq('else', field('else', choice($.if_expression, $.block))))),

    comptime_if_expression: ($) => seq('comptime', 'if', field('condition', $._condition), field('then', $.block), optional(seq('else', field('else', choice($.comptime_if_expression, $.if_expression, $.block))))),

    match_expression: ($) => seq('match', field('value', $._expression), '{', repeat(seq($.match_arm, optional(','))), '}'),

    match_arm: ($) => seq(field('pattern', $._pattern), optional(seq('if', field('guard', $._expression))), '=>', field('body', choice($.block, $._statement))),
  },
});
