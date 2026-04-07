; Tree-sitter highlight queries for the Jade programming language

; ── Comments ──────────────────────────────────────────────────
(comment) @comment.line

; ── Keywords ──────────────────────────────────────────────────
[
  "if"
  "elif"
  "else"
  "while"
  "for"
  "in"
  "loop"
  "match"
  "when"
  "return"
  "break"
  "continue"
  "do"
  "end"
] @keyword

[
  "type"
  "enum"
  "pub"
  "extern"
  "fn"
  "as"
  "to"
  "by"
] @keyword.type

[
  "is"
  "isnt"
  "equals"
  "and"
  "or"
  "not"
] @keyword.operator

; ── Literals ──────────────────────────────────────────────────
(integer) @number
(float) @number.float

(string) @string
(raw_string) @string
(string_content) @string
(escape_sequence) @string.escape

(true) @constant.builtin
(false) @constant.builtin
(none) @constant.builtin

(placeholder) @variable.builtin

; ── Operators ─────────────────────────────────────────────────
(operator) @operator

[
  "~"
  "?"
  "!"
] @operator

; ── Punctuation ───────────────────────────────────────────────
["(" ")"] @punctuation.bracket
["[" "]"] @punctuation.bracket
[","] @punctuation.delimiter
["."] @punctuation.delimiter
[":"] @punctuation.delimiter

; ── Functions ─────────────────────────────────────────────────
(function_definition
  "*" @keyword.function
  name: (identifier) @function)

(lambda_expression
  "|" @keyword.function)

(call_expression
  function: (identifier) @function.call)

(method_expression
  method: (identifier) @function.method.call)

(log_expression
  "log" @function.builtin)

(parameter
  name: (identifier) @variable.parameter)

; ── Type definitions ──────────────────────────────────────────
(type_definition
  name: (identifier) @type)

(enum_definition
  name: (identifier) @type)

(variant_definition
  name: (identifier) @type.enummember)

(type_annotation) @type

(function_type) @type

; ── Patterns ──────────────────────────────────────────────────
(constructor_pattern
  name: (identifier) @type)

(wildcard_pattern) @variable.builtin

; ── Bindings & Fields ─────────────────────────────────────────
(binding
  name: (identifier) @variable)

(field_definition
  name: (identifier) @property)

(field_init
  name: (identifier) @variable.parameter)

(member_expression
  property: (identifier) @property)

(cast_expression
  "as" @keyword.operator)
