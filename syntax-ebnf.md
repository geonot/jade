# Jade EBNF Grammar (Post-Audit Target Syntax)
# This describes the target grammar after all syntax changes are applied.
# Changes from current grammar are marked with # CHANGED

# ═══════════════════════════════════════════════════════════════
# Top-level
# ═══════════════════════════════════════════════════════════════

program        = { item | NEWLINE } ;
item           = function_def | type_def | enum_def | extern_def
               | store_def | actor_def | trait_def | impl_def
               | err_def | use_decl | statement ;

# ═══════════════════════════════════════════════════════════════
# Declarations
# ═══════════════════════════════════════════════════════════════

function_def   = "*" IDENT [ "(" param_list ")" | param_list ]
                 [ "returns" type ]                           # CHANGED: was "->"
                 ( "is" expr                                  # inline body
                 | NEWLINE block ) ;

param_list     = param { "," param } ;
param          = IDENT [ "as" type ] [ "is" expr ] ;          # CHANGED: was ":"

extern_def     = "extern" "*" IDENT
                 "(" param_list ")"
                 [ "returns" type ]                            # CHANGED: was "->"
                 [ "..." ] ;

type_def       = [ "pub" ] "type" IDENT [ "of" type_params ]
                 [ layout_attrs ]
                 NEWLINE INDENT { field_def | function_def | NEWLINE } DEDENT ;

field_def      = IDENT [ "as" type ] [ "is" expr ] NEWLINE ;  # CHANGED: was ":"

type_params    = IDENT { "," IDENT } ;

layout_attrs   = { "@packed" | "@strict" | "@align(" INT ")" } ;

enum_def       = "enum" IDENT [ "of" type_params ]
                 NEWLINE INDENT { variant_def | NEWLINE } DEDENT ;

variant_def    = IDENT [ type_list | "(" type_list ")" ]       # CHANGED: parens optional
                 NEWLINE ;
type_list      = type { "," type } ;

err_def        = "err" IDENT
                 NEWLINE INDENT { IDENT [ "(" type_list ")" ] NEWLINE } DEDENT ;

store_def      = "store" IDENT
                 NEWLINE INDENT { field_def } DEDENT ;

actor_def      = "actor" IDENT
                 NEWLINE INDENT { field_def | function_def | NEWLINE } DEDENT ;

trait_def      = "trait" IDENT [ "of" type_params ]
                 NEWLINE INDENT { function_def | NEWLINE } DEDENT ;

impl_def       = "impl" IDENT "of" type_params "for" IDENT
                 NEWLINE INDENT { function_def | NEWLINE } DEDENT ;

use_decl       = "use" IDENT { "." IDENT } ;

# ═══════════════════════════════════════════════════════════════
# Types
# ═══════════════════════════════════════════════════════════════

type           = "%" type                                      # pointer
               | IDENT [ "of" type { "," type } ]              # generic: Vec of String
               | fn_type ;

fn_type        = "(" type_list ")" "returns" type ;            # CHANGED: was "->"

# ═══════════════════════════════════════════════════════════════
# Statements
# ═══════════════════════════════════════════════════════════════

statement      = binding
               | if_stmt | while_stmt | for_stmt | loop_stmt
               | match_stmt
               | return_stmt | break_stmt | continue_stmt
               | expr_stmt ;

binding        = IDENT [ "as" type ] "is" expr NEWLINE ;       # CHANGED: was ":"
if_stmt        = "if" expr NEWLINE block { elif_clause } [ else_clause ] ;
elif_clause    = "elif" expr NEWLINE block ;
else_clause    = "else" NEWLINE block ;
while_stmt     = "while" expr NEWLINE block ;
for_stmt       = "for" IDENT ("in" | "from") expr
                 [ "to" expr ] [ "by" expr ]
                 [ "if" expr ]
                 NEWLINE block ;
loop_stmt      = "loop" NEWLINE block ;
match_stmt     = "match" expr NEWLINE match_block ;
match_block    = INDENT { match_arm | NEWLINE } DEDENT ;
match_arm      = pattern "?" ( expr NEWLINE | NEWLINE block ) ;
return_stmt    = "return" [ expr ] NEWLINE ;
break_stmt     = "break" [ expr ] NEWLINE ;
continue_stmt  = "continue" NEWLINE ;
expr_stmt      = expr NEWLINE ;

block          = INDENT { statement | NEWLINE } DEDENT ;

# ═══════════════════════════════════════════════════════════════
# Patterns
# ═══════════════════════════════════════════════════════════════

pattern        = "_"
               | IDENT [ "(" pattern_list ")" ]
               | literal
               | pattern "or" pattern
               | pattern "to" pattern ;

pattern_list   = pattern { "," pattern } ;

# ═══════════════════════════════════════════════════════════════
# Expressions
# ═══════════════════════════════════════════════════════════════

expr           = ternary_expr ;

ternary_expr   = pipeline_expr [ "?" expr "!" expr ] ;

pipeline_expr  = or_expr { "~" or_expr } ;

or_expr        = and_expr { "or" and_expr } ;
and_expr       = eq_expr { "and" eq_expr } ;

eq_expr        = cmp_expr { ( "equals" | "eq"                 # CHANGED: added aliases
                             | "isnt" | "neq" ) cmp_expr } ;

cmp_expr       = bitor_expr { ( "<" | ">" | "<=" | ">="
                              | "lt" | "gt" | "lte" | "gte"   # CHANGED: added word forms
                              ) bitor_expr } ;

bitor_expr     = bitxor_expr { "|" bitxor_expr } ;
bitxor_expr    = bitand_expr { "^" bitand_expr } ;
bitand_expr    = shift_expr { "&" shift_expr } ;
shift_expr     = add_expr { ( "<<" | ">>" ) add_expr } ;

add_expr       = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr       = exp_expr { ( "*" | "/" | "mod" ) exp_expr } ; # CHANGED: "%" → "mod"

exp_expr       = unary_expr [ "**" exp_expr ] ;                # right-assoc

unary_expr     = ( "-" | "not" ) unary_expr
               | postfix_expr ;

postfix_expr   = primary { call_suffix | member_suffix | index_suffix | cast_suffix } ;

call_suffix    = "(" [ arg_list ] ")" ;
member_suffix  = "." IDENT [ "(" [ arg_list ] ")" ] ;
index_suffix   = "[" expr "]"
               | "at" expr ;                                   # CHANGED: added "at"
cast_suffix    = "as" type ;

arg_list       = arg { "," arg } ;
arg            = [ IDENT "is" ] expr ;                         # named: x is 10

# ═══════════════════════════════════════════════════════════════
# Lambda
# ═══════════════════════════════════════════════════════════════

lambda         = "*" "fn" "(" [ param_list ] ")"
                 [ "returns" type ]                            # CHANGED: was "->"
                 expr ;

# ═══════════════════════════════════════════════════════════════
# Primary Expressions
# ═══════════════════════════════════════════════════════════════

primary        = INT | FLOAT | STRING | "true" | "false" | "none"
               | IDENT
               | "$" [ INT ]                                   # placeholder
               | "(" expr ")"                                  # grouping
               | "(" expr "," [ expr_list ] ")"                # tuple
               | "[" [ expr_list ] "]"                         # array
               | "[" expr "for" IDENT ("in"|"from") expr       # comprehension
                     [ "to" expr ] [ "if" expr ] "]"
               | "log" expr                                    # log (parens optional)
               | "vec" [ "(" ")" ]                             # vec constructor
               | "map" [ "(" ")" ]                             # map constructor
               | lambda
               | if_expr ;

if_expr        = "if" expr NEWLINE block
                 { elif_clause }
                 [ else_clause ] ;

expr_list      = expr { "," expr } ;

# ═══════════════════════════════════════════════════════════════
# Literals
# ═══════════════════════════════════════════════════════════════

literal        = INT | FLOAT | STRING | "true" | "false" | "none" ;

INT            = digit { digit | "_" }
               | "0x" hex_digit { hex_digit | "_" }
               | "0b" bin_digit { bin_digit | "_" }
               | "0o" oct_digit { oct_digit | "_" } ;

FLOAT          = digit { digit } "." digit { digit }
                 [ ( "e" | "E" ) [ "+" | "-" ] digit { digit } ] ;

STRING         = "'" { char | escape | "{" expr "}" } "'"      # interpolated
               | '"' { char | escape } '"' ;                   # raw

# ═══════════════════════════════════════════════════════════════
# Keywords (complete set)
# ═══════════════════════════════════════════════════════════════
# 
# Control:    if elif else while for in to by from loop break continue
#             return match yield
#
# Binding:    is
#
# Logic:      and or not
#
# Equality:   equals eq isnt neq                                # CHANGED: added eq/neq
#
# Compare:    lt gt lte gte                                     # CHANGED: new word forms
#
# Arithmetic: mod                                               # CHANGED: new
#
# Types:      type enum err as of pub                           # "as" serves dual role
#
# Functions:  fn extern returns                                 # CHANGED: added "returns"
#
# Index:      at                                                # CHANGED: new
#
# Values:     true false none
#
# Actors:     actor spawn send receive dispatch channel
#             close select stop timeout default
#
# Store:      store insert delete set transaction query
#
# Other:      use trait impl test embed assert asm
#             unsafe volatile signal weak log array
#
# Pipeline:   ~ (tilde)
# Ternary:    ? (question) ! (bang)
# Pointer:    % (prefix) & (address-of) @ (deref)
# Function:   * (prefix)
