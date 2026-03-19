#include "tree_sitter/parser.h"
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>

/*
 * External scanner for Jade's indentation-sensitive grammar.
 * Emits INDENT / DEDENT / NEWLINE / WS_NEWLINE tokens.
 */

enum TokenType {
  INDENT,
  DEDENT,
  NEWLINE,
  WS_NEWLINE,
};

#define MAX_DEPTH 128

typedef struct {
  uint16_t indent_stack[MAX_DEPTH];
  uint8_t  depth;
  uint8_t  queued_dedents;
  bool     pending_indent;
  bool     eof_done;
} Scanner;

static void init(Scanner *s) {
  s->indent_stack[0] = 0;
  s->depth = 1;
  s->queued_dedents = 0;
  s->pending_indent = false;
  s->eof_done = false;
}

void *tree_sitter_jade_external_scanner_create(void) {
  Scanner *s = calloc(1, sizeof(Scanner));
  init(s);
  return s;
}

void tree_sitter_jade_external_scanner_destroy(void *p) { free(p); }

unsigned tree_sitter_jade_external_scanner_serialize(void *p, char *buf) {
  Scanner *s = p;
  unsigned i = 0;
  buf[i++] = s->depth;
  buf[i++] = s->queued_dedents;
  buf[i++] = s->pending_indent;
  buf[i++] = s->eof_done;
  for (uint8_t j = 0; j < s->depth && i + 2 <= TREE_SITTER_SERIALIZATION_BUFFER_SIZE; j++) {
    buf[i++] = s->indent_stack[j] & 0xFF;
    buf[i++] = (s->indent_stack[j] >> 8) & 0xFF;
  }
  return i;
}

void tree_sitter_jade_external_scanner_deserialize(void *p, const char *buf, unsigned len) {
  Scanner *s = p;
  if (len == 0) { init(s); return; }
  unsigned i = 0;
  s->depth          = (uint8_t)buf[i++];
  s->queued_dedents = (uint8_t)buf[i++];
  s->pending_indent = buf[i++] != 0;
  s->eof_done       = buf[i++] != 0;
  for (uint8_t j = 0; j < s->depth && i + 2 <= len; j++) {
    s->indent_stack[j] = (uint16_t)((unsigned char)buf[i] | ((unsigned char)buf[i+1] << 8));
    i += 2;
  }
}

static uint16_t cur_indent(Scanner *s) {
  return s->indent_stack[s->depth - 1];
}

bool tree_sitter_jade_external_scanner_scan(void *p, TSLexer *lex, const bool *valid) {
  Scanner *s = p;

  // 1. Drain queued DEDENTs
  if (s->queued_dedents > 0) {
    if (valid[DEDENT]) { s->queued_dedents--; lex->result_symbol = DEDENT; return true; }
    if (valid[NEWLINE]) { lex->result_symbol = NEWLINE; return true; }
  }

  // 2. Pending INDENT
  if (s->pending_indent) {
    if (valid[INDENT]) { s->pending_indent = false; lex->result_symbol = INDENT; return true; }
    if (s->depth > 1) s->depth--;
    s->pending_indent = false;
  }

  // 3. If NEWLINE not valid, emit WS_NEWLINE for bracket interiors
  if (!valid[NEWLINE]) {
    if (valid[WS_NEWLINE] && !lex->eof(lex) &&
        (lex->lookahead == '\n' || lex->lookahead == '\r')) {
      if (lex->lookahead == '\r') lex->advance(lex, false);
      if (!lex->eof(lex) && lex->lookahead == '\n') lex->advance(lex, false);
      lex->mark_end(lex);
      lex->result_symbol = WS_NEWLINE;
      return true;
    }
    return false;
  }

  // 4. EOF: final newline + dedents
  if (lex->eof(lex)) {
    if (s->eof_done) return false;
    s->eof_done = true;
    while (s->depth > 1) { s->depth--; s->queued_dedents++; }
    lex->result_symbol = NEWLINE;
    lex->mark_end(lex);
    return true;
  }

  // 5. Skip trailing whitespace before newline
  while (!lex->eof(lex) && (lex->lookahead == ' ' || lex->lookahead == '\t'))
    lex->advance(lex, true);

  if (lex->eof(lex) || (lex->lookahead != '\n' && lex->lookahead != '\r'))
    return false;

  lex->result_symbol = NEWLINE;

  // Consume newline
  if (lex->lookahead == '\r') lex->advance(lex, false);
  if (!lex->eof(lex) && lex->lookahead == '\n') lex->advance(lex, false);
  lex->mark_end(lex);

  // Skip blank lines, measure indent of next content line
  uint16_t indent = 0;
  for (;;) {
    indent = 0;
    while (!lex->eof(lex) && (lex->lookahead == ' ' || lex->lookahead == '\t')) {
      indent += (lex->lookahead == '\t') ? 4 : 1;
      lex->advance(lex, true);
    }
    if (lex->eof(lex)) {
      while (s->depth > 1) { s->depth--; s->queued_dedents++; }
      return true;
    }
    if (lex->lookahead == '\n' || lex->lookahead == '\r') {
      if (lex->lookahead == '\r') lex->advance(lex, false);
      if (!lex->eof(lex) && lex->lookahead == '\n') lex->advance(lex, false);
      lex->mark_end(lex);
      continue;
    }
    if (lex->lookahead == '#') {
      // Comment line: skip entire line, treat as blank
      while (!lex->eof(lex) && lex->lookahead != '\n' && lex->lookahead != '\r')
        lex->advance(lex, false);
      lex->mark_end(lex);
      continue;
    }
    break;
  }

  uint16_t cur = cur_indent(s);
  if (indent > cur) {
    if (s->depth < MAX_DEPTH) s->indent_stack[s->depth++] = indent;
    s->pending_indent = true;
  } else if (indent < cur) {
    while (s->depth > 1 && s->indent_stack[s->depth - 1] > indent) {
      s->depth--;
      s->queued_dedents++;
    }
  }

  return true;
}
