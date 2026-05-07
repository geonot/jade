/*
 * Full-text search runtime for Jade.
 *
 * Provides a basic inverted index for @search-annotated string fields.
 * Supports insert (add document), search (query terms), and count.
 *
 * File format: Simple flat file with entries:
 *   [4B term_len][term bytes][8B doc_id] per posting
 * This is a naive but functional implementation — suitable for small-medium datasets.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <ctype.h>
#include "jade_rt.h"

#define FTS_MAGIC "JADEFTS\0"

struct JadeFts {
    FILE   *fp;
    char    path[256];
    int64_t posting_count;
};

/* ── Open / Close ─────────────────────────────────────────── */

JadeFts *jade_fts_open(const char *path) {
    JadeFts *f = (JadeFts *)calloc(1, sizeof(JadeFts));
    if (!f) return NULL;
    strncpy(f->path, path, 255);

    f->fp = fopen(path, "r+b");
    if (f->fp) {
        char magic[8];
        fread(magic, 1, 8, f->fp);
        fread(&f->posting_count, 8, 1, f->fp);
    } else {
        f->fp = fopen(path, "w+b");
        if (!f->fp) { free(f); return NULL; }
        fwrite(FTS_MAGIC, 1, 8, f->fp);
        f->posting_count = 0;
        fwrite(&f->posting_count, 8, 1, f->fp);
        fflush(f->fp);
    }
    return f;
}

void jade_fts_close(JadeFts *f) {
    if (!f) return;
    if (f->fp) fclose(f->fp);
    free(f);
}

/* ── Tokenization ─────────────────────────────────────────── */

/* Simple whitespace+punctuation tokenizer. Lowercases tokens. */
static int next_token(const char *text, int pos, char *tok, int max_tok) {
    int len = (int)strlen(text);
    /* skip non-alpha */
    while (pos < len && !isalnum((unsigned char)text[pos])) pos++;
    if (pos >= len) return -1;

    int ti = 0;
    while (pos < len && isalnum((unsigned char)text[pos]) && ti < max_tok - 1) {
        tok[ti++] = (char)tolower((unsigned char)text[pos]);
        pos++;
    }
    tok[ti] = '\0';
    return pos;
}

/* ── Index a document ─────────────────────────────────────── */

void jade_fts_add(JadeFts *f, int64_t doc_id, const char *text, int64_t text_len) {
    if (!f || !f->fp || !text) return;

    char tok[128];
    int pos = 0;
    int64_t added = 0;
    while ((pos = next_token(text, pos, tok, 128)) >= 0) {
        int32_t tlen = (int32_t)strlen(tok);
        if (tlen == 0) continue;

        /* Seek to end of file and write posting */
        if (fseek(f->fp, 0, SEEK_END) != 0 ||
            fwrite(&tlen, 4, 1, f->fp) != 1 ||
            fwrite(tok, 1, tlen, f->fp) != (size_t)tlen ||
            fwrite(&doc_id, 8, 1, f->fp) != 1) {
            fprintf(stderr, "jade: fts: write posting failed\n");
            break;
        }
        added++;
    }

    f->posting_count += added;
    /* Update posting count in header */
    if (fseek(f->fp, 8, SEEK_SET) != 0 ||
        fwrite(&f->posting_count, 8, 1, f->fp) != 1) {
        fprintf(stderr, "jade: fts: failed to update posting count\n");
    }
    fflush(f->fp);
}

/* ── Search ───────────────────────────────────────────────── */

/* Search for a single term. Returns count of matching doc_ids.
 * If out_ids is non-NULL, fills up to max_ids matching doc_ids. */
int64_t jade_fts_search(JadeFts *f, const char *query, int64_t *out_ids, int64_t max_ids) {
    if (!f || !f->fp || !query) return 0;

    /* Lowercase the query term */
    char lquery[128];
    int qi = 0;
    for (int i = 0; query[i] && qi < 127; i++) {
        lquery[qi++] = (char)tolower((unsigned char)query[i]);
    }
    lquery[qi] = '\0';
    int32_t qlen = (int32_t)strlen(lquery);

    /* Scan all postings (naive linear scan) */
    fseek(f->fp, 16, SEEK_SET);  /* skip magic + count */
    int64_t found = 0;
    char tok[128];

    while (1) {
        int32_t tlen;
        if (fread(&tlen, 4, 1, f->fp) != 1) break;
        if (tlen <= 0 || tlen > 127) break;
        if (fread(tok, 1, tlen, f->fp) != (size_t)tlen) break;
        tok[tlen] = '\0';

        int64_t doc_id;
        if (fread(&doc_id, 8, 1, f->fp) != 1) break;

        if (tlen == qlen && memcmp(tok, lquery, qlen) == 0) {
            /* Deduplicate: don't add same doc_id twice */
            int dup = 0;
            for (int64_t i = 0; i < found && i < max_ids; i++) {
                if (out_ids && out_ids[i] == doc_id) { dup = 1; break; }
            }
            if (!dup) {
                if (out_ids && found < max_ids) {
                    out_ids[found] = doc_id;
                }
                found++;
            }
        }
    }
    return found;
}

/* Count documents matching a query term */
int64_t jade_fts_count(JadeFts *f, const char *query) {
    return jade_fts_search(f, query, NULL, 0);
}

/* ── Convenience wrappers for Jade string type (data + len) ── */

int64_t jade_fts_search_n(JadeFts *f, const char *query, int64_t qlen) {
    /* Null-terminate a copy */
    char buf[256];
    if (qlen > 255) qlen = 255;
    memcpy(buf, query, qlen);
    buf[qlen] = '\0';
    return jade_fts_search(f, buf, NULL, 0);
}

int64_t jade_fts_count_n(JadeFts *f, const char *query, int64_t qlen) {
    return jade_fts_search_n(f, query, qlen);
}

void jade_fts_add_n(JadeFts *f, int64_t doc_id, const char *text, int64_t text_len) {
    char buf[4096];
    if (text_len > 4095) text_len = 4095;
    memcpy(buf, text, text_len);
    buf[text_len] = '\0';
    jade_fts_add(f, doc_id, buf, text_len);
}

int64_t jade_fts_posting_count(JadeFts *f) {
    if (!f) return 0;
    return f->posting_count;
}
