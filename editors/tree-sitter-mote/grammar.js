/**
 * Tree-sitter grammar for Mote, written for editor highlighting. The compiler's parser is the
 * reference: newlines are insignificant inside brackets there, so they are plain whitespace here.
 */

const PREC = {
  ternary: 1,
  coalesce: 2,
  or: 3,
  and: 4,
  bit_or: 5,
  bit_xor: 6,
  bit_and: 7,
  equality: 8,
  compare: 9,
  type_test: 10,
  shift: 11,
  add: 12,
  multiply: 13,
  unary: 14,
  range: 15,
  postfix: 16,
};

const sep1 = (rule, separator) => seq(rule, repeat(seq(separator, rule)));
const commaSep = (rule) => optional(seq(sep1(rule, ','), optional(',')));
const commaSep1 = (rule) => seq(sep1(rule, ','), optional(','));

module.exports = grammar({
  name: 'mote',

  word: ($) => $.identifier,

  extras: ($) => [/\s/, $.line_comment, $.doc_comment, $.block_comment],

  supertypes: ($) => [$._expression, $._type, $._pattern, $._statement],

  conflicts: ($) => [
    [$.ternary_expression, $.try_expression],
    [$.range_pattern, $.field_pattern],
    [$.nullable_type, $.union_type],
    [$.or_pattern],
    [$.or_pattern, $.range_pattern],
    [$.at_pattern, $.or_pattern],
    [$.at_pattern, $.range_pattern],
    [$.tuple_type, $.function_type],
    [$._type, $.nullable_type],
    [$._type, $.union_type],
    [$.block, $.map_expression],
    [$._expression, $.struct_expression],
    [$._expression, $.struct_expression, $.generic_expression],
    [$._expression, $.generic_expression],
    [$.test_declaration, $._expression],
    [$.pattern_identifier, $.enum_pattern],
  ],

  rules: {
    source_file: ($) => repeat($._item),

    _item: ($) =>
      choice(
        $.attribute,
        $.import_declaration,
        $.function_item,
        $.native_function,
        $.struct_item,
        $.class_item,
        $.enum_item,
        $.trait_item,
        $.impl_item,
        $.type_alias,
        $.test_declaration,
        $._statement,
      ),

    // ---- comments -------------------------------------------------------

    line_comment: () => token(prec(1, /\/\/[^\n]*/)),
    doc_comment: () => token(prec(2, /\/\/\/[^\n]*/)),
    // Not nested: the compiler nests `/* /* */ */`, a regex cannot.
    block_comment: () => token(/\/\*[^*]*\*+([^/*][^*]*\*+)*\//),

    // ---- attributes and visibility --------------------------------------

    attribute: ($) =>
      prec.right(seq('@', field('name', $.identifier), optional(seq('(', commaSep($.identifier), ')')))),

    visibility: () => 'pub',

    // ---- imports --------------------------------------------------------

    import_declaration: ($) =>
      seq(
        optional($.visibility),
        'import',
        choice(
          seq($.import_list, choice('from', 'in'), field('path', $.module_path)),
          seq(
            field('path', $.module_path),
            optional(seq('from', field('from', $.module_path))),
            optional(seq('as', field('alias', $.identifier))),
          ),
        ),
      ),

    import_list: ($) => seq('{', choice('*', commaSep($.import_symbol)), '}'),

    import_symbol: ($) =>
      seq(field('name', $.identifier), optional(seq('as', field('alias', $.identifier)))),

    module_path: ($) =>
      seq(
        repeat(choice('.', '..')),
        repeat(seq($.super, '.')),
        $.identifier,
        repeat(seq('.', $.identifier)),
      ),

    super: () => 'super',

    // ---- declarations ---------------------------------------------------

    function_item: ($) =>
      seq(
        optional($.visibility),
        'fn',
        field('name', $._member_name),
        optional($.type_parameters),
        field('parameters', $.parameters),
        optional(seq('->', field('return_type', $._type))),
        field('body', $.block),
      ),

    native_function: ($) =>
      seq(
        optional($.visibility),
        'native',
        'fn',
        field('name', $.identifier),
        field('parameters', $.parameters),
        optional(seq('->', field('return_type', $._type))),
      ),

    _member_name: ($) => choice($.identifier, alias('from', $.identifier)),

    type_parameters: ($) => seq('<', commaSep1($.type_parameter), '>'),

    type_parameter: ($) =>
      seq(
        field('name', $.identifier),
        optional(seq(':', field('bounds', sep1($._type, '+')))),
      ),

    parameters: ($) => seq('(', commaSep($._parameter), ')'),

    _parameter: ($) => choice($.self_parameter, $.variadic_parameter, $.parameter),

    self_parameter: () => seq(optional('var'), 'self'),

    variadic_parameter: ($) =>
      seq('...', field('name', $.identifier), optional(seq(':', field('type', $._type)))),

    parameter: ($) =>
      seq(
        optional('var'),
        field('name', $.identifier),
        optional(seq(':', field('type', $._type))),
        optional(seq('=', field('default', $._expression))),
      ),

    struct_item: ($) =>
      seq(
        optional($.visibility),
        'struct',
        field('name', $.identifier),
        optional($.type_parameters),
        field('body', $.member_list),
      ),

    class_item: ($) =>
      seq(
        optional($.visibility),
        'class',
        field('name', $.identifier),
        optional($.type_parameters),
        optional(seq(':', field('parent', $.identifier))),
        field('body', $.member_list),
      ),

    member_list: ($) => seq('{', repeat(choice($.field_declaration, $.function_item, $.attribute)), '}'),

    field_declaration: ($) =>
      seq(
        optional($.visibility),
        optional(choice('var', 'let')),
        field('name', $.identifier),
        ':',
        field('type', $._type),
      ),

    enum_item: ($) =>
      seq(
        optional($.visibility),
        'enum',
        field('name', $.identifier),
        optional($.type_parameters),
        '{',
        repeat(seq($.enum_variant, optional(','))),
        '}',
      ),

    enum_variant: ($) =>
      seq(
        field('name', $.identifier),
        optional(
          choice(
            seq('(', commaSep($._type), ')'),
            seq('{', repeat(seq($.variant_field, optional(','))), '}'),
            seq('=', field('value', $._expression)),
          ),
        ),
      ),

    variant_field: ($) =>
      seq(optional(choice('var', 'let')), field('name', $.identifier), ':', field('type', $._type)),

    trait_item: ($) =>
      seq(
        optional($.visibility),
        'trait',
        field('name', $.identifier),
        optional($.type_parameters),
        '{',
        repeat(choice($.trait_method, $.attribute)),
        '}',
      ),

    trait_method: ($) =>
      seq(
        'fn',
        field('name', $.identifier),
        optional($.type_parameters),
        field('parameters', $.parameters),
        optional(seq('->', field('return_type', $._type))),
        optional(field('body', $.block)),
      ),

    impl_item: ($) =>
      seq(
        'impl',
        optional($.type_parameters),
        field('trait', $._type),
        optional(seq('for', field('type', $._type))),
        '{',
        repeat(choice($.function_item, $.attribute)),
        '}',
      ),

    type_alias: ($) =>
      seq(
        optional($.visibility),
        'type',
        field('name', $.identifier),
        optional($.type_parameters),
        '=',
        field('type', $._type),
      ),

    test_declaration: ($) => seq(field('keyword', $.identifier), $.string, $.block),

    // ---- types ----------------------------------------------------------

    _type: ($) => choice($._operand_type, $.union_type),

    _operand_type: ($) =>
      choice(
        $.named_type,
        $.generic_type,
        $.self_type,
        $.tuple_type,
        $.function_type,
        $.array_type,
        $.nullable_type,
      ),

    named_type: ($) => prec.right(seq($.identifier, repeat(seq('.', $.identifier)))),

    generic_type: ($) =>
      seq(
        alias(prec.right(seq($.identifier, repeat(seq('.', $.identifier)))), $.named_type),
        $.type_arguments,
      ),

    type_arguments: ($) => seq('<', commaSep1($._type), '>'),

    self_type: () => 'Self',

    tuple_type: ($) => seq('(', commaSep($._type), ')'),

    function_type: ($) =>
      prec.right(
        seq(
          optional(alias('Send', $.send_marker)),
          '(',
          commaSep(seq(optional('var'), $._type)),
          ')',
          '->',
          field('return_type', $._type),
        ),
      ),

    array_type: ($) => seq('[', $._type, optional(seq(';', $._expression)), ']'),

    nullable_type: ($) => seq($._operand_type, choice('?', '??')),

    union_type: ($) => prec.left(seq($._operand_type, repeat1(seq('|', $._operand_type)))),

    // ---- statements -----------------------------------------------------

    block: ($) => seq('{', repeat($._statement), '}'),

    _statement: ($) =>
      choice(
        $.let_declaration,
        $.tuple_let_declaration,
        $.if_statement,
        $.while_statement,
        $.for_statement,
        $.match_statement,
        $.scope_statement,
        $.with_statement,
        $.return_statement,
        $.break_statement,
        $.continue_statement,
        $.yield_statement,
        $.assignment,
        $.expression_statement,
        $.empty_statement,
      ),

    empty_statement: () => ';',

    let_declaration: ($) =>
      seq(
        optional($.visibility),
        field('keyword', choice('let', 'var')),
        field('name', $.identifier),
        optional(seq(':', field('type', $._type))),
        '=',
        field('value', $._expression),
      ),

    tuple_let_declaration: ($) =>
      seq(
        optional($.visibility),
        choice('let', 'var'),
        '(',
        sep1($.identifier, ','),
        ')',
        '=',
        field('value', $._expression),
      ),

    if_statement: ($) =>
      seq(
        'if',
        field('condition', $._expression),
        field('consequence', $.block),
        optional(seq('else', field('alternative', choice($.if_statement, $.block)))),
      ),

    while_statement: ($) =>
      seq('while', field('condition', $._expression), field('body', $.block)),

    for_statement: ($) =>
      seq(
        'for',
        field('variable', $.identifier),
        'in',
        field('iterable', $._expression),
        field('body', $.block),
      ),

    match_statement: ($) =>
      seq('match', field('subject', $._expression), '{', repeat($.match_arm), '}'),

    match_arm: ($) =>
      seq(
        field('pattern', $._pattern),
        optional(seq('if', field('guard', $._expression))),
        '=>',
        field('body', choice($.block, $._statement)),
        optional(','),
      ),

    scope_statement: ($) => seq('scope', $.block),

    with_statement: ($) =>
      seq('with', field('name', $.identifier), '=', field('value', $._expression), $.block),

    return_statement: ($) => prec.right(seq('return', optional($._expression))),

    break_statement: ($) => prec.right(seq('break', optional($._expression))),

    continue_statement: () => 'continue',

    yield_statement: ($) => seq('yield', $._expression),

    assignment: ($) =>
      seq(
        field('left', $._expression),
        field(
          'operator',
          choice(
            '=',
            '+=',
            '-=',
            '*=',
            '/=',
            '%=',
            '&=',
            '|=',
            '^=',
            '<<=',
            seq('>', token.immediate('>=')),
            '??=',
          ),
        ),
        field('right', $._expression),
      ),

    expression_statement: ($) => seq($._expression),

    // ---- patterns -------------------------------------------------------

    _pattern: ($) =>
      choice(
        $.wildcard_pattern,
        $.literal_pattern,
        $.pattern_identifier,
        $.at_pattern,
        $.tuple_pattern,
        $.or_pattern,
        $.range_pattern,
        $.enum_pattern,
        $.struct_pattern,
        $.type_pattern,
      ),

    wildcard_pattern: () => '_',

    literal_pattern: ($) =>
      choice($.integer, seq('-', $.integer), $.string, $.boolean, $.null, $.float, $.char),

    pattern_identifier: ($) => $.identifier,

    at_pattern: ($) => seq(field('name', $.identifier), '@', field('pattern', $._pattern)),

    tuple_pattern: ($) => seq('(', commaSep($._pattern), ')'),

    or_pattern: ($) => prec.left(seq($._pattern, repeat1(seq('|', $._pattern)))),

    range_pattern: ($) =>
      prec.left(seq($._pattern, choice('..', '..='), $._pattern)),

    enum_pattern: ($) =>
      seq(
        field('path', seq($.identifier, repeat(seq('.', $.identifier)))),
        optional(seq('(', commaSep($._pattern), ')')),
      ),

    struct_pattern: ($) =>
      seq(
        field('path', seq($.identifier, repeat(seq('.', $.identifier)))),
        '{',
        repeat(seq(choice($.field_pattern, '..'), optional(','))),
        '}',
      ),

    field_pattern: ($) =>
      seq(field('name', $.identifier), optional(seq(':', field('pattern', $._pattern)))),

    type_pattern: ($) =>
      seq(field('name', choice($.identifier, '_')), ':', field('type', $._operand_type)),

    // ---- expressions ----------------------------------------------------

    _expression: ($) =>
      choice(
        $.integer,
        $.float,
        $.string,
        $.char,
        $.boolean,
        $.null,
        $.identifier,
        $.self,
        $.parenthesized_expression,
        $.tuple_expression,
        $.list_expression,
        $.map_expression,
        $.struct_expression,
        $.generic_expression,
        $.unary_expression,
        $.binary_expression,
        $.type_test,
        $.range_expression,
        $.ternary_expression,
        $.coalesce_expression,
        $.try_expression,
        $.unwrap_expression,
        $.call_expression,
        $.member_expression,
        $.index_expression,
        $.lambda_expression,
        $.spawn_expression,
      ),

    self: () => 'self',

    parenthesized_expression: ($) => seq('(', $._expression, ')'),

    tuple_expression: ($) =>
      choice(seq('(', ')'), seq('(', $._expression, ',', commaSep($._expression), ')')),

    list_expression: ($) => seq('[', commaSep($._expression), ']'),

    map_expression: ($) => seq('{', commaSep($.map_entry), '}'),

    map_entry: ($) => seq(field('key', $._expression), ':', field('value', $._expression)),

    struct_expression: ($) =>
      seq(
        field('name', seq($.identifier, repeat(seq('.', $.identifier)))),
        '{',
        repeat(seq($.field_initializer, optional(','))),
        '}',
      ),

    field_initializer: ($) => seq(field('name', $.identifier), ':', field('value', $._expression)),

    // `Name<Args>(…)` and `Name<Args>.member`: type arguments written in an expression.
    generic_expression: ($) =>
      prec.dynamic(
        1,
        seq(
          field('name', seq($.identifier, repeat(seq('.', $.identifier)))),
          $.type_arguments,
        ),
      ),

    unary_expression: ($) =>
      prec(PREC.unary, seq(field('operator', choice('-', '!', '~', 'try')), field('operand', $._expression))),

    binary_expression: ($) => {
      const table = [
        [PREC.or, '||'],
        [PREC.and, '&&'],
        [PREC.bit_or, '|'],
        [PREC.bit_xor, '^'],
        [PREC.bit_and, '&'],
        [PREC.equality, choice('==', '!=')],
        [PREC.compare, choice('<', '<=', '>', '>=')],
        [PREC.shift, choice('<<', seq('>', token.immediate('>')))],
        [PREC.add, choice('+', '-')],
        [PREC.multiply, choice('*', '/', '%')],
      ];
      return choice(
        ...table.map(([precedence, operator]) =>
          prec.left(
            precedence,
            seq(field('left', $._expression), field('operator', operator), field('right', $._expression)),
          ),
        ),
      );
    },

    type_test: ($) =>
      prec.left(PREC.type_test, seq($._expression, 'is', field('type', $._operand_type))),

    range_expression: ($) =>
      prec.left(
        PREC.range,
        seq(field('start', $._expression), choice('..', '..='), field('end', $._expression)),
      ),

    ternary_expression: ($) =>
      prec.right(
        PREC.ternary,
        seq(
          field('condition', $._expression),
          '?',
          field('consequence', $._expression),
          ':',
          field('alternative', $._expression),
        ),
      ),

    coalesce_expression: ($) =>
      prec.left(PREC.coalesce, seq($._expression, '??', $._expression)),

    // Same level as the ternary so GLR keeps both readings of `?`; a later `:` decides.
    try_expression: ($) => prec(PREC.ternary, seq($._expression, '?')),

    unwrap_expression: ($) => prec(PREC.postfix, seq($._expression, '!')),

    call_expression: ($) =>
      prec(PREC.postfix, seq(field('function', $._expression), field('arguments', $.arguments))),

    arguments: ($) => seq('(', commaSep(choice($._expression, $.named_argument)), ')'),

    // `name = value` in a call.
    named_argument: ($) => seq(field('name', $.identifier), '=', field('value', $._expression)),

    member_expression: ($) =>
      prec(
        PREC.postfix,
        seq(
          field('object', $._expression),
          field('operator', choice('.', '?.')),
          field('property', choice($.identifier, alias('from', $.identifier), $.tuple_index)),
        ),
      ),

    tuple_index: () => token.immediate(/[0-9]+(\.[0-9]+)*/),

    index_expression: ($) =>
      prec(PREC.postfix, seq(field('object', $._expression), '[', field('index', $._expression), ']')),

    lambda_expression: ($) =>
      prec.right(
        seq(
          choice(seq('|', commaSep($.lambda_parameter), '|'), '||'),
          optional(seq('->', field('return_type', $._type))),
          field('body', choice($.block, $._expression)),
        ),
      ),

    lambda_parameter: ($) =>
      seq(optional('var'), field('name', $.identifier), optional(seq(':', field('type', $._operand_type)))),

    spawn_expression: ($) =>
      prec.right(seq('spawn', choice($.block, prec(90, $._expression)))),

    // ---- literals -------------------------------------------------------

    boolean: () => choice('true', 'false'),

    null: () => 'null',

    integer: () =>
      token(
        choice(
          /0[xX][0-9a-fA-F_]+(_?[ui](8|16|32|64|size))?/,
          /0[bB][01_]+(_?[ui](8|16|32|64|size))?/,
          /0[oO][0-7_]+(_?[ui](8|16|32|64|size))?/,
          /[0-9][0-9_]*(_?[ui](8|16|32|64|size))?/,
        ),
      ),

    float: () =>
      token(
        choice(
          /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9_]+)?(_?f(32|64))?/,
          /[0-9][0-9_]*[eE][+-]?[0-9_]+(_?f(32|64))?/,
        ),
      ),

    char: () => token(/'([^'\\]|\\([^\n]|u\{[0-9a-fA-F]+\}))*'/),

    string: ($) =>
      seq(
        '"',
        repeat(
          choice(
            alias(token.immediate(prec(1, /[^"\\$]+/)), $.string_content),
            $.escape_sequence,
            $.interpolation,
            alias(token.immediate('$'), $.string_content),
          ),
        ),
        '"',
      ),

    escape_sequence: () => token.immediate(/\\([^u]|u\{[0-9a-fA-F]+\})/),

    interpolation: ($) => seq(token.immediate('${'), $._expression, '}'),

    identifier: () => /[a-zA-Z_][a-zA-Z0-9_]*/,
  },
});
