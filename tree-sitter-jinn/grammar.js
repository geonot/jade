/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

// Tree-sitter grammar for the Jinn programming language.
//
// Jinn is indentation-sensitive. The external scanner (src/scanner.c)
// emits NEWLINE / INDENT / DEDENT tokens (plus WS_NEWLINE for blank or
// comment-only lines, which is consumed as whitespace).
//
// This grammar is a faithful model of the reference parser in
// `src/parser/` and the authoritative EBNF in `jinn.ebnf`. It is verified
// against the conformance corpus in `tests/ebnf_corpus/*.jn`: every snippet
// must parse with zero ERROR nodes (`tree-sitter parse --quiet`).
//
// Lexical notes confirmed against `src/lexer/`:
//   * `"..."`  -> raw string  (no escapes, no interpolation)   [lex_raw_string]
//   * `'...'`  -> rich string (escapes + `{expr}` interpolation, `'''` heredoc) [lex_string]
//   * `#`      -> line comment
//   * `self`, `vec` are ordinary identifiers (NOT reserved keywords).
//   * word operators (`gt lt lte gte eq equals neq and or xor not mod pow in`)
//     are reserved keywords lexed to the same tokens as their symbolic forms.

const PREC = {
  TERNARY: 1,
  PIPELINE: 2,
  OR: 3,
  XOR: 4,
  AND: 5,
  EQUALITY: 6,
  COMPARE: 7,
  BIT_OR: 8,
  BIT_XOR: 9,
  BIT_AND: 10,
  SHIFT: 11,
  ADD: 12,
  MUL: 13,
  EXP: 14,
  UNARY: 15,
  CALL: 16,
  MEMBER: 17,
};

// Helper: a left-associative binary expression at a given precedence.
function binLeft($, precedence, operatorRule) {
  return prec.left(
    precedence,
    seq(
      field('left', $._expression),
      field('operator', operatorRule),
      field('right', $._expression),
    ),
  );
}

module.exports = grammar({
  name: 'jinn',

  word: $ => $.identifier,

  externals: $ => [$._indent, $._dedent, $._newline, $._ws_newline],

  extras: $ => [/[ \r]+/, $.comment, $._ws_newline],

  conflicts: $ => [
    // `( ident ...` is ambiguous between a lambda parameter list, a
    // parenthesised / tuple expression, and a destructuring tuple binding
    // until a later token (`=>`, `)`, `is`) disambiguates.
    [$.parameter, $._primary_expression, $.tuple_binding],
    [$.parameter, $._primary_expression],
    // `if` shares identical block syntax as both a statement and an
    // expression; prefer the statement form at statement position.
    [$.if_statement, $.if_expression],
    // `Name <` — decide between a bare named type and the start of a
    // generic argument list; GLR resolves once `>` (or its absence) is seen.
    [$.named_type],
  ],

  supertypes: $ => [$._declaration, $._statement, $._expression, $._type, $._pattern],

  rules: {
    source_file: $ => repeat(choice($._newline, $._declaration)),

    comment: _ => token(seq('#', /[^\n]*/)),

    // ── Declarations ────────────────────────────────────────────────────
    _declaration: $ => choice(
      $.annotated_function,
      $.function_definition,
      $.extern_definition,
      $.type_definition,
      $.enum_definition,
      $.error_definition,
      $.trait_definition,
      $.impl_definition,
      $.actor_definition,
      $.store_definition,
      $.view_definition,
      $.migration_definition,
      $.use_declaration,
      $.test_definition,
      $.const_declaration,
      $.global_declaration,
      $.alias_declaration,
    ),

    attribute: $ => seq(
      '@',
      field('name', $.identifier),
      optional(seq('(', optional($._argument_list), ')')),
    ),

    annotated_function: $ => seq(
      repeat1(seq($.attribute, $._newline)),
      $.function_definition,
    ),

    function_definition: $ => seq(
      '*',
      field('name', $.identifier),
      optional($.type_parameters),
      optional($.parameter_list),
      optional(seq('returns', field('return_type', $._type))),
      repeat(seq('!', field('error_type', $._type))),
      $._newline,
      field('body', $.block),
    ),

    extern_definition: $ => seq(
      'extern',
      '*',
      field('name', $.identifier),
      optional($.parameter_list),
      optional(seq('returns', field('return_type', $._type))),
      $._newline,
    ),

    type_parameters: $ => seq(
      'of',
      commaSep1($.type_parameter),
    ),

    type_parameter: $ => seq(
      field('name', $.identifier),
      optional(seq(':', sep1('+', $._type))),
    ),

    parameter_list: $ => choice(
      seq('(', optional(commaSep1($.parameter)), ')'),
      // paren-less parameter list, e.g. `*greet name as string`
      commaSep1($.parameter),
    ),

    parameter: $ => seq(
      field('name', $.identifier),
      optional(seq('as', field('type', $._type))),
      optional(seq('is', field('default', $._expression))),
    ),

    type_definition: $ => seq(
      optional('pub'),
      'type',
      field('name', $.identifier),
      optional($.type_parameters),
      optional($.attribute),
      $._newline,
      field('body', $.type_body),
    ),

    type_body: $ => seq(
      $._indent,
      repeat(choice(
        seq($.field_definition, $._newline),
        $.function_definition,
        $.annotated_function,
        $._newline,
      )),
      $._dedent,
    ),

    field_definition: $ => seq(
      optional('&'),
      field('name', $.identifier),
      optional(seq('as', field('type', $._type))),
      optional(seq('is', field('default', $._expression))),
    ),

    enum_definition: $ => seq(
      optional('pub'),
      'enum',
      field('name', $.identifier),
      optional($.type_parameters),
      $._newline,
      $._indent,
      repeat(choice(seq($.variant_definition, $._newline), $._newline)),
      $._dedent,
    ),

    variant_definition: $ => seq(
      field('name', $.identifier),
      optional(seq('(', commaSep1($._type), ')')),
    ),

    error_definition: $ => seq(
      'err',
      field('name', $.identifier),
      $._newline,
      $._indent,
      repeat(choice(seq($.variant_definition, $._newline), $._newline)),
      $._dedent,
    ),

    trait_definition: $ => seq(
      'trait',
      field('name', $.identifier),
      optional($.type_parameters),
      $._newline,
      field('body', $.block),
    ),

    impl_definition: $ => seq(
      'impl',
      optional(seq(field('trait', $.identifier), 'for')),
      field('type', $._type),
      $._newline,
      field('body', $.block),
    ),

    actor_definition: $ => seq(
      'actor',
      field('name', $.identifier),
      $._newline,
      $._indent,
      repeat(choice(
        seq($.field_definition, $._newline),
        $.message_handler,
        $._newline,
      )),
      $._dedent,
    ),

    message_handler: $ => choice(
      // `*loop [sleep_expr]` periodic handler
      seq(
        '*',
        'loop',
        optional(field('interval', $._expression)),
        $._newline,
        field('body', $.block),
      ),
      // `@name params` (async) or `*name params` (sync)
      seq(
        choice('@', '*'),
        field('name', $.identifier),
        optional($.handler_parameters),
        optional(seq('returns', field('return_type', $._type))),
        $._newline,
        field('body', $.block),
      ),
    ),

    handler_parameters: $ => choice(
      seq('(', optional(commaSep1($.handler_parameter)), ')'),
      commaSep1($.handler_parameter),
    ),

    handler_parameter: $ => seq(
      field('name', $.identifier),
      optional(seq('as', field('type', $._type))),
    ),

    store_definition: $ => seq(
      'store',
      field('name', $.identifier),
      repeat($.attribute),
      $._newline,
      $._indent,
      repeat(choice(
        seq(repeat($.attribute), $.field_definition, $._newline),
        $.function_definition,
        $._newline,
      )),
      $._dedent,
    ),

    view_definition: $ => seq(
      'view',
      field('name', $.identifier),
      'of',
      field('source', $.identifier),
      $._newline,
      field('body', $.block),
    ),

    migration_definition: $ => seq(
      'migration',
      field('name', $.identifier),
      $._newline,
      field('body', $.block),
    ),

    use_declaration: $ => seq(
      'use',
      field('path', $.use_path),
      optional(seq('as', field('alias', $.identifier))),
      $._newline,
    ),

    use_path: $ => sep1('.', $.identifier),

    test_definition: $ => seq(
      'test',
      field('name', $.raw_string),
      $._newline,
      field('body', $.block),
    ),

    const_declaration: $ => seq(
      optional('const'),
      field('name', $.identifier),
      optional(seq('as', field('type', $._type))),
      'is',
      field('value', $._expression),
      $._newline,
    ),

    global_declaration: $ => seq(
      'global',
      field('name', $.identifier),
      optional(seq('as', field('type', $._type))),
      'is',
      field('value', $._expression),
      $._newline,
    ),

    alias_declaration: $ => seq(
      'alias',
      field('name', $.identifier),
      'is',
      field('type', $._type),
      $._newline,
    ),

    // ── Types ───────────────────────────────────────────────────────────
    _type: $ => choice(
      $.pointer_type,
      $.collection_type,
      $.tuple_type,
      $.named_type,
    ),

    pointer_type: $ => prec(2, seq('%', $._type)),

    collection_type: $ => prec.right(seq(
      field('kind', choice('vec', 'channel', $.identifier)),
      'of',
      field('element', $._type),
    )),

    tuple_type: $ => seq('(', commaSep1($._type), ')'),

    named_type: $ => seq(
      field('name', $.identifier),
      optional($.type_arguments),
    ),

    type_arguments: $ => seq('<', commaSep1($._type), '>'),

    // ── Block & statements ──────────────────────────────────────────────
    block: $ => seq(
      $._indent,
      repeat(choice($._statement, $._newline)),
      $._dedent,
    ),

    _statement: $ => choice(
      $.binding,
      $.tuple_binding,
      $.atomic_binding,
      $.assignment,
      $.if_statement,
      $.unless_statement,
      $.while_statement,
      $.until_statement,
      $.for_statement,
      $.sim_statement,
      $.loop_statement,
      $.match_statement,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.defer_statement,
      $.stop_statement,
      $.close_statement,
      $.nop_statement,
      $.use_declaration,
      $.expression_statement,
    ),

    binding: $ => seq(
      field('name', $.identifier),
      optional(seq('as', field('type', $._type))),
      'is',
      field('value', $._expression),
      $._newline,
    ),

    tuple_binding: $ => seq(
      '(',
      commaSep1($.identifier),
      ')',
      'is',
      field('value', $._expression),
      $._newline,
    ),

    atomic_binding: $ => seq(
      'atomic',
      field('name', $.identifier),
      choice('is', field('operator', $.augmented_operator)),
      field('value', $._expression),
      $._newline,
    ),

    assignment: $ => seq(
      choice(
        // bare identifier targets only take an augmented operator; plain
        // `ident is expr` is a `binding`.
        seq(field('target', $.identifier), field('operator', $.augmented_operator)),
        seq(
          field('target', choice($.member_expression, $.index_expression)),
          choice('is', field('operator', $.augmented_operator)),
        ),
      ),
      field('value', $._expression),
      $._newline,
    ),

    augmented_operator: _ => choice(
      '+=', '-=', '*=', '/=', '&=', '|=', '^=', '<<=', '>>=', '>>>=',
    ),

    if_statement: $ => prec.dynamic(1, seq(
      'if',
      field('condition', $._expression),
      $._newline,
      field('consequence', $.block),
      repeat($.elif_clause),
      optional($.else_clause),
    )),

    elif_clause: $ => seq(
      'elif',
      field('condition', $._expression),
      $._newline,
      field('consequence', $.block),
    ),

    else_clause: $ => seq(
      'else',
      $._newline,
      field('consequence', $.block),
    ),

    unless_statement: $ => seq(
      'unless',
      field('condition', $._expression),
      $._newline,
      field('consequence', $.block),
      optional($.else_clause),
    ),

    while_statement: $ => seq(
      'while',
      field('condition', $._expression),
      $._newline,
      field('body', $.block),
    ),

    until_statement: $ => seq(
      'until',
      field('condition', $._expression),
      $._newline,
      field('body', $.block),
    ),

    for_statement: $ => seq(
      'for',
      optional($.access_modifier),
      field('binding', $.identifier),
      optional(seq(',', field('binding2', $.identifier))),
      choice('in', 'from'),
      field('iterable', $._expression),
      optional(seq('to', field('end', $._expression))),
      optional(seq('by', field('step', $._expression))),
      $._newline,
      field('body', $.block),
    ),

    sim_statement: $ => seq(
      'sim',
      choice(
        $.for_statement,
        seq($._newline, field('body', $.block)),
      ),
    ),

    loop_statement: $ => seq(
      'loop',
      optional(seq(
        field('start', $._expression),
        optional(seq('to', field('end', $._expression))),
        optional(seq('by', field('step', $._expression))),
      )),
      $._newline,
      field('body', $.block),
    ),

    access_modifier: _ => choice('copy', 'ref', 'move', 'borrow'),

    match_statement: $ => seq(
      'match',
      field('subject', $._expression),
      $._newline,
      $._indent,
      repeat(choice($.match_arm, $._newline)),
      $._dedent,
    ),

    match_arm: $ => seq(
      field('pattern', $._pattern),
      optional($.match_guard),
      '?',
      choice(
        $._statement,
        seq($._newline, field('body', $.block)),
      ),
    ),

    match_guard: $ => seq(choice('when', 'if'), field('guard', $._pipe_expression)),

    return_statement: $ => seq(
      'return',
      optional(field('value', $._expression)),
      $._newline,
    ),

    break_statement: $ => seq(
      'break',
      optional(field('value', $._expression)),
      $._newline,
    ),

    continue_statement: $ => seq('continue', optional($.identifier), $._newline),

    nop_statement: $ => seq('nop', $._newline),

    defer_statement: $ => seq(
      'defer',
      choice(
        seq($._newline, field('body', $.block)),
        field('body', $._statement),
      ),
    ),

    stop_statement: $ => seq('stop', field('target', $._expression), $._newline),

    close_statement: $ => seq('close', field('channel', $._expression), $._newline),

    expression_statement: $ => seq($._expression, $._newline),

    // ── Expressions ─────────────────────────────────────────────────────
    _expression: $ => choice(
      $.ternary_expression,
      $._pipe_expression,
    ),

    ternary_expression: $ => prec.right(PREC.TERNARY, seq(
      field('condition', $._pipe_expression),
      '?',
      field('consequence', $._pipe_expression),
      optional(seq('!', field('alternative', $._expression))),
    )),

    _pipe_expression: $ => choice(
      $.pipe_expression,
      $._binary_expression,
    ),

    pipe_expression: $ => prec.left(PREC.PIPELINE, seq(
      field('left', $._pipe_expression),
      '~',
      field('right', $._binary_expression),
    )),

    _binary_expression: $ => choice(
      $.binary_expression,
      $._unary_expression,
    ),

    binary_expression: $ => choice(
      binLeft($, PREC.OR, 'or'),
      binLeft($, PREC.XOR, 'xor'),
      binLeft($, PREC.AND, 'and'),
      binLeft($, PREC.EQUALITY, choice('==', '!=', 'equals', 'eq', 'neq')),
      binLeft($, PREC.COMPARE, choice(
        '<', '>', '<=', '>=',
        'lt', 'gt', 'lte', 'gte', 'nlt', 'ngt', 'nlte', 'ngte', 'in',
      )),
      binLeft($, PREC.BIT_OR, '|'),
      binLeft($, PREC.BIT_XOR, '^'),
      binLeft($, PREC.BIT_AND, '&'),
      binLeft($, PREC.SHIFT, choice('<<', '>>', '>>>')),
      binLeft($, PREC.ADD, choice('+', '-')),
      binLeft($, PREC.MUL, choice('*', '/', '%', 'mod')),
      prec.right(PREC.EXP, seq(
        field('left', $._expression),
        field('operator', choice('**', 'pow')),
        field('right', $._expression),
      )),
    ),

    _unary_expression: $ => choice(
      $.unary_expression,
      $.reference_expression,
      $._postfix_expression,
    ),

    unary_expression: $ => prec(PREC.UNARY, seq(
      field('operator', choice('-', 'not', '~')),
      field('operand', $._unary_expression),
    )),

    reference_expression: $ => prec(PREC.UNARY, seq('%', field('operand', $._postfix_expression))),

    _postfix_expression: $ => choice(
      $.call_expression,
      $.method_expression,
      $.member_expression,
      $.index_expression,
      $._primary_expression,
    ),

    call_expression: $ => prec(PREC.CALL, seq(
      field('function', $._postfix_expression),
      '(',
      optional($._argument_list),
      ')',
    )),

    method_expression: $ => prec(PREC.MEMBER + 1, seq(
      field('object', $._postfix_expression),
      '.',
      field('method', $.identifier),
      '(',
      optional($._argument_list),
      ')',
    )),

    member_expression: $ => prec(PREC.MEMBER, seq(
      field('object', $._postfix_expression),
      '.',
      field('property', $.identifier),
    )),

    index_expression: $ => prec(PREC.CALL, seq(
      field('object', $._postfix_expression),
      '[',
      field('index', $._expression),
      ']',
    )),

    _argument_list: $ => commaSep1($.argument),

    argument: $ => choice(
      seq(field('name', $.identifier), 'is', field('value', $._expression)),
      $._expression,
    ),

    _primary_expression: $ => choice(
      $.integer,
      $.float,
      $.string,
      $.raw_string,
      $.boolean,
      $.none,
      $.placeholder,
      $.unreachable,
      $.identifier,
      $.parenthesized_expression,
      $.tuple_expression,
      $.array_expression,
      $.lambda_expression,
      $.spawn_expression,
      $.channel_expression,
      $.receive_expression,
      $.send_expression,
      $.yield_expression,
      $.select_expression,
      $.dispatch_expression,
      $.grad_expression,
      $.einsum_expression,
      $.syscall_expression,
      $.log_expression,
      $.builder_expression,
      $.if_expression,
    ),

    parenthesized_expression: $ => prec(1, seq('(', $._expression, ')')),

    tuple_expression: $ => seq(
      '(',
      $._expression,
      ',',
      optional(commaSep1($._expression)),
      optional(','),
      ')',
    ),

    array_expression: $ => seq(
      '[',
      optional(seq(
        commaSep1($._expression),
        optional(','),
      )),
      ']',
    ),

    lambda_expression: $ => prec(1, seq(
      '(',
      optional(commaSep1($.parameter)),
      ')',
      '=>',
      choice(
        field('body', $._expression),
        seq($._newline, field('body', $.block)),
      ),
    )),

    spawn_expression: $ => prec.right(seq(
      'spawn',
      field('actor', sep1('.', $.identifier)),
      optional(seq('(', optional(commaSep1($.field_init)), ')')),
    )),

    field_init: $ => seq(
      field('name', $.identifier),
      'is',
      field('value', $._expression),
    ),

    channel_expression: $ => prec.right(seq(
      'channel',
      optional(seq('of', field('element', $._type))),
      optional(seq('(', field('capacity', $._expression), ')')),
    )),

    receive_expression: $ => prec.right(seq(
      'receive',
      choice(
        field('channel', $._expression),
        seq($._newline, $._indent, repeat1($.receive_arm), $._dedent),
      ),
    )),

    receive_arm: $ => seq(
      '@',
      field('handler', $.identifier),
      optional(seq('(', optional(commaSep1($.identifier)), ')')),
      $._newline,
      field('body', $.block),
    ),

    send_expression: $ => prec.right(seq(
      'send',
      field('target', $._expression),
      ',',
      choice(
        seq('@', field('handler', $.identifier), optional(seq('(', optional($._argument_list), ')'))),
        field('value', $._expression),
      ),
    )),

    yield_expression: $ => prec.right(seq('yield', optional(field('value', $._expression)))),

    select_expression: $ => seq(
      'select',
      $._newline,
      $._indent,
      repeat1(choice($.select_arm, $._newline)),
      $._dedent,
    ),

    select_arm: $ => seq(
      choice(
        seq('send', field('channel', $._expression), ',', field('value', $._expression)),
        seq('receive', field('channel', $.identifier), optional(seq('as', field('binding', $.identifier)))),
        'default',
      ),
      $._newline,
      field('body', $.block),
    ),

    dispatch_expression: $ => prec.right(seq(
      'dispatch',
      choice(
        seq(optional($.identifier), $._newline, field('body', $.block)),
        seq(field('target', $._expression), ',', '@', field('handler', $.identifier),
          optional(seq('(', optional($._argument_list), ')'))),
      ),
    )),

    grad_expression: $ => seq('grad', '(', field('argument', $._expression), ')'),

    einsum_expression: $ => seq(
      'einsum',
      '(',
      field('spec', choice($.string, $.raw_string)),
      ',',
      commaSep1($._expression),
      ')',
    ),

    syscall_expression: $ => seq('syscall', '(', optional($._argument_list), ')'),

    log_expression: $ => seq('log', '(', optional($._argument_list), ')'),

    builder_expression: $ => seq(
      'build',
      field('type', $.identifier),
      $._newline,
      $._indent,
      repeat1(seq($.field_init, $._newline)),
      $._dedent,
    ),

    if_expression: $ => prec.right(seq(
      'if',
      field('condition', $._expression),
      $._newline,
      field('consequence', $.block),
      repeat($.elif_clause),
      optional($.else_clause),
    )),

    // ── Patterns ────────────────────────────────────────────────────────
    _pattern: $ => choice(
      $.wildcard_pattern,
      $.literal_pattern,
      $.constructor_pattern,
      $.tuple_pattern,
      $.identifier_pattern,
    ),

    wildcard_pattern: _ => '_',

    literal_pattern: $ => choice($.integer, $.float, $.string, $.raw_string, $.boolean, $.none),

    identifier_pattern: $ => $.identifier,

    constructor_pattern: $ => seq(
      field('name', $.identifier),
      '(',
      commaSep1($._pattern),
      ')',
    ),

    tuple_pattern: $ => seq('(', commaSep1($._pattern), ')'),

    // ── Literals ────────────────────────────────────────────────────────
    integer: _ => token(choice(
      /0[xX][0-9a-fA-F_]+/,
      /0[bB][01_]+/,
      /0[oO][0-7_]+/,
      /[0-9][0-9_]*/,
    )),

    float: _ => token(choice(
      /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9]+)?/,
      /[0-9][0-9_]*[eE][+-]?[0-9]+/,
    )),

    boolean: _ => choice('true', 'false'),

    none: _ => 'none',

    unreachable: _ => 'unreachable',

    placeholder: _ => choice('$', '$$'),

    // Double-quoted raw string: no escapes, no interpolation.
    raw_string: _ => seq('"', /[^"\n]*/, '"'),

    // Single-quoted rich string: escapes + `{expr}` interpolation, and
    // triple-quoted heredocs.
    string: $ => choice(
      seq(
        "'''",
        repeat(choice(
          $.escape_sequence,
          token.immediate(prec(1, /[^'\\]+/)),
          token.immediate(prec(1, /'[^']/)),
          token.immediate("''"),
        )),
        "'''",
      ),
      seq(
        "'",
        repeat(choice(
          $.interpolation,
          $.escape_sequence,
          $.string_content,
        )),
        "'",
      ),
    ),

    string_content: _ => token.immediate(prec(1, /[^'\\{]+/)),

    escape_sequence: _ => token.immediate(/\\['"\\ntr0{}]/),

    interpolation: $ => seq(
      token.immediate('{'),
      $._expression,
      '}',
    ),

    identifier: _ => /[A-Za-z_][A-Za-z0-9_]*/,
  },
});

// ── helpers ────────────────────────────────────────────────────────────
function commaSep1(rule) {
  return seq(rule, repeat(seq(',', rule)));
}

function sep1(separator, rule) {
  return seq(rule, repeat(seq(separator, rule)));
}
