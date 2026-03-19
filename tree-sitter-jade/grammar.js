/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

// Tree-sitter grammar for the Jade programming language.
//
// Jade is indentation-sensitive. The external scanner handles
// NEWLINE, INDENT, DEDENT tokens. Inside (...) and [...], newlines
// are treated as whitespace via _ws_newline in extras.

const PREC = {
  PIPELINE: 1,
  TERNARY: 2,
  OR: 3,
  AND: 4,
  EQUALITY: 5,
  COMPARE: 6,
  BIT_OR: 7,
  BIT_XOR: 8,
  BIT_AND: 9,
  SHIFT: 10,
  ADD: 11,
  MUL: 12,
  EXP: 13,
  UNARY: 14,
  CALL: 15,
  MEMBER: 16,
};

function commaSep1(rule) {
  return seq(rule, repeat(seq(",", rule)));
}

function commaSep(rule) {
  return optional(commaSep1(rule));
}

module.exports = grammar({
  name: "jade",

  extras: ($) => [/[ \t\r]/, $.comment, $._ws_newline],

  externals: ($) => [$._indent, $._dedent, $._newline, $._ws_newline],

  word: ($) => $.identifier,

  conflicts: ($) => [
    [$.method_expression, $.member_expression],
    [$.if_statement, $.if_expression],
  ],

  rules: {
    source_file: ($) => repeat(choice($._item, $._newline)),

    // ── Top-level items ──────────────────────────────────────
    _item: ($) =>
      choice(
        $.function_definition,
        $.type_definition,
        $.enum_definition,
        $._statement,
      ),

    // ── Function definition ──────────────────────────────────
    function_definition: ($) =>
      seq(
        "*",
        field("name", $.identifier),
        "(",
        optional(field("parameters", $.parameter_list)),
        ")",
        optional(seq("->", field("return_type", $.type_annotation))),
        $._newline,
        field("body", $.block),
      ),

    parameter_list: ($) => commaSep1($.parameter),

    parameter: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq(":", field("type", $.type_annotation))),
        optional(seq("is", field("default", $._expression))),
      ),

    type_annotation: ($) =>
      choice(
        $.identifier,
        $.function_type,
      ),

    function_type: ($) =>
      seq("(", commaSep($.type_annotation), ")", "->", $.type_annotation),

    // ── Type definition ──────────────────────────────────────
    type_definition: ($) =>
      seq(
        optional("pub"),
        "type",
        field("name", $.identifier),
        $._newline,
        optional(field("body", $.type_body)),
      ),

    type_body: ($) =>
      seq(
        $._indent,
        repeat(choice($.field_definition, $.function_definition, $._newline)),
        $._dedent,
      ),

    field_definition: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq(":", field("type", $.type_annotation))),
        optional(seq("is", field("default", $._expression))),
        $._newline,
      ),

    // ── Enum definition ──────────────────────────────────────
    enum_definition: ($) =>
      seq(
        "enum",
        field("name", $.identifier),
        $._newline,
        optional(field("body", $.enum_body)),
      ),

    enum_body: ($) =>
      seq(
        $._indent,
        repeat(choice($.variant_definition, $._newline)),
        $._dedent,
      ),

    variant_definition: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq("(", optional(commaSep1($.variant_field)), ")")),
        $._newline,
      ),

    variant_field: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq("as", field("type", $.type_annotation))),
      ),

    // ── Block ────────────────────────────────────────────────
    block: ($) =>
      seq(
        $._indent,
        repeat(choice($._statement, $._newline)),
        $._dedent,
      ),

    // ── Statements ───────────────────────────────────────────
    _statement: ($) =>
      choice(
        $.binding,
        $.if_statement,
        $.while_statement,
        $.for_statement,
        $.loop_statement,
        $.match_statement,
        $.return_statement,
        $.break_statement,
        $.continue_statement,
        $.expression_statement,
      ),

    binding: ($) =>
      seq(
        field("name", $.identifier),
        "is",
        field("value", $._expression),
        $._newline,
      ),

    if_statement: ($) =>
      seq(
        "if",
        field("condition", $._expression),
        $._newline,
        field("body", $.block),
        repeat($.elif_clause),
        optional($.else_clause),
      ),

    elif_clause: ($) =>
      seq("elif", field("condition", $._expression), $._newline, field("body", $.block)),

    else_clause: ($) =>
      seq("else", $._newline, field("body", $.block)),

    while_statement: ($) =>
      seq(
        "while",
        field("condition", $._expression),
        $._newline,
        field("body", $.block),
      ),

    for_statement: ($) =>
      seq(
        "for",
        field("variable", $.identifier),
        "in",
        field("iterable", $._expression),
        optional(seq("to", field("end", $._expression))),
        optional(seq("by", field("step", $._expression))),
        $._newline,
        field("body", $.block),
      ),

    loop_statement: ($) =>
      seq("loop", $._newline, field("body", $.block)),

    match_statement: ($) =>
      seq(
        "match",
        field("subject", $._expression),
        $._newline,
        field("arms", $.match_block),
      ),

    match_block: ($) =>
      seq($._indent, repeat(choice($.match_arm, $._newline)), $._dedent),

    match_arm: ($) =>
      seq(
        field("pattern", $.pattern),
        "?",
        choice(
          seq(field("body", $._expression), $._newline),
          seq($._newline, field("body", $.block)),
        ),
      ),

    return_statement: ($) =>
      seq("return", optional(field("value", $._expression)), $._newline),

    break_statement: ($) =>
      seq("break", optional(field("value", $._expression)), $._newline),

    continue_statement: ($) =>
      seq("continue", $._newline),

    expression_statement: ($) =>
      seq($._expression, $._newline),

    // ── Patterns ─────────────────────────────────────────────
    pattern: ($) =>
      choice(
        $.wildcard_pattern,
        $.constructor_pattern,
        $.literal_pattern,
        $.identifier_pattern,
      ),

    wildcard_pattern: (_$) => "_",

    constructor_pattern: ($) =>
      seq(
        field("name", $.identifier),
        "(",
        optional(commaSep1($.pattern)),
        ")",
      ),

    literal_pattern: ($) =>
      choice($.integer, $.float, $.string, $.true, $.false, $.none),

    identifier_pattern: ($) => $.identifier,

    // ── Expressions ──────────────────────────────────────────
    _expression: ($) =>
      choice(
        $.ternary_expression,
        $.pipeline_expression,
        $.binary_expression,
        $.unary_expression,
        $.call_expression,
        $.method_expression,
        $.member_expression,
        $.index_expression,
        $.cast_expression,
        $.lambda_expression,
        $.if_expression,
        $._primary_expression,
      ),

    ternary_expression: ($) =>
      prec.right(PREC.TERNARY, seq(
        field("condition", $._expression),
        "?",
        field("consequence", $._expression),
        "!",
        field("alternative", $._expression),
      )),

    pipeline_expression: ($) =>
      prec.left(PREC.PIPELINE, seq(
        field("left", $._expression),
        "~",
        field("right", $._expression),
      )),

    binary_expression: ($) => {
      const table = [
        ["or", PREC.OR],
        ["and", PREC.AND],
        ["equals", PREC.EQUALITY],
        ["isnt", PREC.EQUALITY],
        [">", PREC.COMPARE],
        [">=", PREC.COMPARE],
        ["<", PREC.COMPARE],
        ["<=", PREC.COMPARE],
        ["|", PREC.BIT_OR],
        ["^", PREC.BIT_XOR],
        ["&", PREC.BIT_AND],
        ["<<", PREC.SHIFT],
        [">>", PREC.SHIFT],
        ["+", PREC.ADD],
        ["-", PREC.ADD],
        ["*", PREC.MUL],
        ["/", PREC.MUL],
        ["%", PREC.MUL],
        ["**", PREC.EXP],
      ];
      return choice(
        ...table.map(([op, p]) => {
          const assoc = op === "**" ? prec.right : prec.left;
          return assoc(p, seq(
            field("left", $._expression),
            field("operator", alias(op, $.operator)),
            field("right", $._expression),
          ));
        }),
      );
    },

    unary_expression: ($) =>
      prec(PREC.UNARY, choice(
        seq("-", field("operand", $._expression)),
        seq("not", field("operand", $._expression)),
      )),

    call_expression: ($) =>
      prec(PREC.CALL, seq(
        field("function", $._expression),
        "(",
        optional(field("arguments", $.argument_list)),
        ")",
      )),

    argument_list: ($) =>
      commaSep1(choice($.field_init, $._expression)),

    field_init: ($) =>
      seq(field("name", $.identifier), "is", field("value", $._expression)),

    method_expression: ($) =>
      prec(PREC.MEMBER, seq(
        field("object", $._expression),
        ".",
        field("method", $.identifier),
        "(",
        optional(field("arguments", $.argument_list)),
        ")",
      )),

    member_expression: ($) =>
      prec(PREC.MEMBER, seq(
        field("object", $._expression),
        ".",
        field("property", $.identifier),
      )),

    index_expression: ($) =>
      prec(PREC.CALL, seq(
        field("object", $._expression),
        "[",
        field("index", $._expression),
        "]",
      )),

    cast_expression: ($) =>
      prec(PREC.CALL, seq(
        field("value", $._expression),
        "as",
        field("type", $.type_annotation),
      )),

    lambda_expression: ($) =>
      choice(
        // *fn(x) expr
        seq("*", "fn", "(", optional(field("parameters", $.parameter_list)), ")",
          optional(seq("->", field("return_type", $.type_annotation))),
          field("body", $._expression)),
        // *fn(x) do ... end
        seq("*", "fn", "(", optional(field("parameters", $.parameter_list)), ")",
          optional(seq("->", field("return_type", $.type_annotation))),
          "do",
          field("body", $.do_end_body)),
      ),

    do_end_body: ($) =>
      seq(repeat(choice($._statement, $._newline)), "end"),

    if_expression: ($) =>
      seq(
        "if",
        field("condition", $._expression),
        $._newline,
        field("body", $.block),
        repeat($.elif_clause),
        optional($.else_clause),
      ),

    // ── Primary expressions ──────────────────────────────────
    _primary_expression: ($) =>
      choice(
        $.integer,
        $.float,
        $.string,
        $.raw_string,
        $.true,
        $.false,
        $.none,
        $.identifier,
        $.placeholder,
        $.parenthesized_expression,
        $.tuple_expression,
        $.array_literal,
        $.log_expression,
      ),

    parenthesized_expression: ($) =>
      seq("(", $._expression, ")"),

    tuple_expression: ($) =>
      choice(
        seq("(", ")"),   // unit
        seq("(", $._expression, ",", optional(commaSep1($._expression)), ")"),
      ),

    array_literal: ($) =>
      seq("[", commaSep($._expression), optional(","), "]"),

    log_expression: ($) =>
      seq("log", "(", field("value", $._expression), ")"),

    // ── Literals ─────────────────────────────────────────────
    integer: (_$) =>
      token(choice(
        /0[xX][0-9a-fA-F][0-9a-fA-F_]*/,
        /0[bB][01][01_]*/,
        /0[oO][0-7][0-7_]*/,
        /[0-9][0-9_]*/,
      )),

    float: (_$) =>
      token(choice(
        /[0-9][0-9_]*\.[0-9][0-9_]*/,
        /[0-9][0-9_]*[eE][+-]?[0-9]+/,
        /[0-9][0-9_]*\.[0-9][0-9_]*[eE][+-]?[0-9]+/,
      )),

    string: ($) =>
      seq("'", repeat(choice($.string_content, $.escape_sequence)), "'"),

    string_content: (_$) => token.immediate(prec(1, /[^'\\\n]+/)),

    escape_sequence: (_$) => token.immediate(seq("\\", /[nrt0\\']/)),

    raw_string: (_$) =>
      token(seq('"', repeat(choice(/[^"\\\n]/, /\\./)), '"')),

    true: (_$) => "true",
    false: (_$) => "false",
    none: (_$) => "none",

    placeholder: (_$) => token(choice("$", /\$[0-9]+/)),

    identifier: (_$) => /[a-zA-Z_][a-zA-Z0-9_]*/,

    comment: (_$) => token(seq("#", /.*/)),
  },
});
