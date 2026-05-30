; Tree-sitter highlight queries for the Jinn programming language

; ── Comments ──────────────────────────────────────────────────
(comment) @comment.line

; ── Keywords ──────────────────────────────────────────────────
[
  "if"
  "elif"
  "else"
  "unless"
  "while"
  "until"
  "for"
  "in"
  "loop"
  "sim"
  "match"
  "when"
  "return"
  "break"
  "continue"
  "defer"
  "stop"
  "close"
  "nop"
] @keyword

[
  "type"
  "enum"
  "err"
  "trait"
  "impl"
  "actor"
  "store"
  "view"
  "migration"
  "extern"
  "pub"
  "use"
  "test"
  "const"
  "global"
  "alias"
  "of"
  "as"
  "to"
  "by"
  "from"
] @keyword.type

[
  "spawn"
  "send"
  "receive"
  "channel"
  "yield"
  "dispatch"
  "select"
  "atomic"
] @keyword

[
  "is"
  "and"
  "or"
  "xor"
  "not"
  "equals"
  "eq"
  "neq"
  "lt"
  "gt"
  "lte"
  "gte"
  "nlt"
  "ngt"
  "nlte"
  "ngte"
  "mod"
  "pow"
] @keyword.operator

; ── Literals ──────────────────────────────────────────────────
(integer) @number
(float) @number.float

(string) @string
(raw_string) @string
(string_content) @string
(escape_sequence) @string.escape

(boolean) @constant.builtin
(none) @constant.builtin
(unreachable) @constant.builtin

(placeholder) @variable.builtin

; ── Operators ─────────────────────────────────────────────────
(binary_expression
  operator: _ @operator)

(unary_expression
  operator: _ @operator)

[
  "~"
  "?"
  "!"
  "=>"
  "%"
] @operator

; ── Punctuation ───────────────────────────────────────────────
["(" ")"] @punctuation.bracket
["[" "]"] @punctuation.bracket
["<" ">"] @punctuation.bracket
[","] @punctuation.delimiter
["."] @punctuation.delimiter

; ── Functions ─────────────────────────────────────────────────
(function_definition
  "*" @keyword.function
  name: (identifier) @function)

(call_expression
  function: (identifier) @function.call)

(method_expression
  method: (identifier) @function.method.call)

(log_expression
  "log" @function.builtin)

(grad_expression
  "grad" @function.builtin)

(einsum_expression
  "einsum" @function.builtin)

(syscall_expression
  "syscall" @function.builtin)

(parameter
  name: (identifier) @variable.parameter)

(handler_parameter
  name: (identifier) @variable.parameter)

; ── Attributes ────────────────────────────────────────────────
(attribute
  "@" @attribute
  name: (identifier) @attribute)

(message_handler
  name: (identifier) @function)

; ── Type definitions ──────────────────────────────────────────
(type_definition
  name: (identifier) @type)

(enum_definition
  name: (identifier) @type)

(error_definition
  name: (identifier) @type)

(trait_definition
  name: (identifier) @type)

(actor_definition
  name: (identifier) @type)

(store_definition
  name: (identifier) @type)

(variant_definition
  name: (identifier) @type.enum)

(named_type
  name: (identifier) @type)

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
