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
        $.extern_definition,
        $.actor_definition,
        $.supervisor_definition,
        $.store_definition,
        $.trait_definition,
        $.impl_definition,
        $.err_definition,
        $.use_declaration,
        $.alias_definition,
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
        optional(seq("returns", field("return_type", $.type_annotation))),
        $._newline,
        field("body", $.block),
      ),

    parameter_list: ($) => commaSep1($.parameter),

    parameter: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq("as", field("type", $.type_annotation))),
        optional(seq("is", field("default", $._expression))),
      ),

    type_annotation: ($) =>
      choice(
        $.identifier,
        $.pointer_type,
        $.generic_type,
        $.function_type,
        $.simd_type,
      ),

    pointer_type: ($) =>
      seq("%", $.type_annotation),

    generic_type: ($) =>
      prec.left(seq($.identifier, "of", commaSep1($.type_annotation))),

    function_type: ($) =>
      seq("(", commaSep($.type_annotation), ")", "returns", $.type_annotation),

    simd_type: ($) =>
      seq("SIMD", "of", $.type_annotation, ",", $.integer),

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
        optional(seq("as", field("type", $.type_annotation))),
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
        optional(seq("is", field("discriminant", $.integer))),
        $._newline,
      ),

    variant_field: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq("as", field("type", $.type_annotation))),
      ),

    // ── Extern definition ────────────────────────────────────
    extern_definition: ($) =>
      seq(
        "extern",
        "*",
        field("name", $.identifier),
        "(",
        optional(field("parameters", $.parameter_list)),
        ")",
        optional(seq("returns", field("return_type", $.type_annotation))),
        $._newline,
      ),

    // ── Actor definition ─────────────────────────────────────
    actor_definition: ($) =>
      seq(
        "actor",
        field("name", $.identifier),
        $._newline,
        optional(seq(
          $._indent,
          repeat(choice($.field_definition, $.function_definition, $._newline)),
          $._dedent,
        )),
      ),

    // ── Supervisor definition ─────────────────────────────────
    supervisor_definition: ($) =>
      seq(
        "supervisor",
        field("name", $.identifier),
        $._newline,
        field("body", $.block),
      ),

    // ── Store definition ─────────────────────────────────────
    store_definition: ($) =>
      seq(
        "store",
        field("name", $.identifier),
        $._newline,
        optional(seq(
          $._indent,
          repeat(choice($.field_definition, $._newline)),
          $._dedent,
        )),
      ),

    // ── Trait definition ─────────────────────────────────────
    trait_definition: ($) =>
      seq(
        "trait",
        field("name", $.identifier),
        optional(seq("of", field("type_params", commaSep1($.identifier)))),
        $._newline,
        optional(seq(
          $._indent,
          repeat(choice($.function_definition, $._newline)),
          $._dedent,
        )),
      ),

    // ── Impl definition ──────────────────────────────────────
    impl_definition: ($) =>
      seq(
        "impl",
        field("trait_name", $.identifier),
        "for",
        field("type_name", $.identifier),
        $._newline,
        optional(seq(
          $._indent,
          repeat(choice($.function_definition, $._newline)),
          $._dedent,
        )),
      ),

    // ── Err definition ───────────────────────────────────────
    err_definition: ($) =>
      seq(
        "err",
        field("name", $.identifier),
        $._newline,
        optional(seq(
          $._indent,
          repeat(choice(
            seq(field("variant", $.identifier), optional(seq("(", commaSep1($.type_annotation), ")")), $._newline),
            $._newline,
          )),
          $._dedent,
        )),
      ),

    // ── Use declaration ──────────────────────────────────────
    use_declaration: ($) =>
      seq(
        "use",
        field("path", $.module_path),
        optional(seq(".", "{", commaSep1($.identifier), "}")),
        optional(seq("as", field("alias", $.identifier))),
        $._newline,
      ),

    module_path: ($) =>
      prec.left(seq($.identifier, repeat(seq(".", $.identifier)))),

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
        $.unless_statement,
        $.while_statement,
        $.until_statement,
        $.for_statement,
        $.sim_for_statement,
        $.loop_statement,
        $.match_statement,
        $.return_statement,
        $.break_statement,
        $.continue_statement,
        $.transaction_statement,
        $.use_local_statement,
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

    until_statement: ($) =>
      seq(
        "until",
        field("condition", $._expression),
        $._newline,
        field("body", $.block),
      ),

    unless_statement: ($) =>
      seq(
        "unless",
        field("condition", $._expression),
        $._newline,
        field("body", $.block),
      ),

    for_statement: ($) =>
      seq(
        "for",
        field("variable", $.identifier),
        choice("in", "from"),
        field("iterable", $._expression),
        optional(seq("to", field("end", $._expression))),
        optional(seq("by", field("step", $._expression))),
        optional(seq("if", field("filter", $._expression))),
        $._newline,
        field("body", $.block),
      ),

    sim_for_statement: ($) =>
      seq(
        "sim",
        "for",
        field("variable", $.identifier),
        choice("in", "from"),
        field("iterable", $._expression),
        optional(seq("to", field("end", $._expression))),
        optional(seq("by", field("step", $._expression))),
        $._newline,
        field("body", $.block),
      ),

    loop_statement: ($) =>
      seq("loop", $._newline, field("body", $.block)),

    transaction_statement: ($) =>
      seq("transaction", $._newline, field("body", $.block)),

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
      seq("break", optional(field("label", $.identifier)), $._newline),

    continue_statement: ($) =>
      seq("continue", optional(field("label", $.identifier)), $._newline),

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
        $.slice_expression,
        $.lambda_expression,
        $.if_expression,
        $.select_expression,
        $.spawn_expression,
        $.send_expression,
        $.channel_expression,
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
        ["xor", PREC.XOR],
        ["and", PREC.AND],
        ["equals", PREC.EQUALITY],
        ["eq", PREC.EQUALITY],
        ["neq", PREC.EQUALITY],
        [">", PREC.COMPARE],
        [">=", PREC.COMPARE],
        ["<", PREC.COMPARE],
        ["<=", PREC.COMPARE],
        ["lt", PREC.COMPARE],
        ["gt", PREC.COMPARE],
        ["lte", PREC.COMPARE],
        ["gte", PREC.COMPARE],
        ["nlt", PREC.COMPARE],
        ["ngt", PREC.COMPARE],
        ["ngte", PREC.COMPARE],
        ["nlte", PREC.COMPARE],
        ["in", PREC.COMPARE],
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
        ["mod", PREC.MUL],
        ["pow", PREC.EXP],
      ];
      return choice(
        ...table.map(([op, p]) => {
          const assoc = op === "pow" ? prec.right : prec.left;
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
      prec(PREC.CALL, choice(
        seq(field("value", $._expression), "as", "strict", field("type", $.type_annotation)),
        seq(field("value", $._expression), "as", "json"),
        seq(field("value", $._expression), "as", "map"),
        seq(field("value", $._expression), "as", field("type", $.type_annotation)),
      )),

    slice_expression: ($) =>
      prec(PREC.CALL, seq(
        field("object", $._expression),
        "[",
        optional(field("start", $._expression)),
        "...",
        optional(field("end", $._expression)),
        "]",
      )),

    select_expression: ($) =>
      seq("select", $._newline, field("body", $.select_body)),

    select_body: ($) =>
      seq(repeat1($.select_arm), "end"),

    select_arm: ($) =>
      seq(
        choice($.identifier, "default"),
        "->",
        field("body", $._expression),
        $._newline,
      ),

    spawn_expression: ($) =>
      prec(PREC.UNARY, seq("spawn", field("body", $._expression))),

    send_expression: ($) =>
      prec.right(PREC.COMPARE, seq(
        field("channel", $._expression),
        "<-",
        field("value", $._expression),
      )),

    channel_expression: ($) =>
      seq("channel", "(", optional(field("capacity", $._expression)), ")"),

    lambda_expression: ($) =>
      choice(
        // |x| expr
        seq("|", optional(field("parameters", $.parameter_list)), "|",
          optional(seq("returns", field("return_type", $.type_annotation))),
          field("body", $._expression)),
        // |x| do ... end
        seq("|", optional(field("parameters", $.parameter_list)), "|",
          optional(seq("returns", field("return_type", $.type_annotation))),
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
        $.unreachable,
        $.identifier,
        $.placeholder,
        $.parenthesized_expression,
        $.tuple_expression,
        $.array_literal,
        $.set_literal,
        $.map_literal,
        $.log_expression,
        $.deque_expression,
        $.grad_expression,
        $.einsum_expression,
        $.yield_expression,
        $.build_expression,
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

    set_literal: ($) =>
      seq("set", "(", optional(commaSep1($._expression)), ")"),

    map_literal: ($) =>
      seq("{", commaSep(seq(field("key", $._expression), ":", field("value", $._expression))), optional(","), "}"),

    unreachable: (_$) => "unreachable",

    deque_expression: ($) =>
      seq("deque", "(", optional(commaSep1($._expression)), ")"),

    grad_expression: ($) =>
      seq("grad", "(", field("function", $._expression), ")"),

    einsum_expression: ($) =>
      seq("einsum", field("spec", $.string), ",", commaSep1($._expression)),

    yield_expression: ($) =>
      seq("yield", field("value", $._expression)),

    build_expression: ($) =>
      seq("build", field("name", $.identifier), $._newline, $._indent,
        repeat(seq(field("field_name", $.identifier), "is", field("field_value", $._expression), $._newline)),
      $._dedent),

    alias_definition: ($) =>
      seq("alias", field("name", $.identifier), "is", field("type", $.type_annotation), $._newline),

    use_local_statement: ($) =>
      seq("use", field("path", $.identifier), optional(seq("[", commaSep1($.identifier), "]")), $._newline),

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
