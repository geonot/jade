#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <ctype.h>
#include <errno.h>
#include "jinn_rt.h"
#define FTS_MAGIC "JINNFTS\0"

struct JinnFts {
    FILE   *fp;
    char    path[256];
    int64_t posting_count;
};

JinnFts *jinn_fts_open(const char *path) {
    jinn_persistence_check_path(path);
    JinnFts *f = (JinnFts *)calloc(1, sizeof(JinnFts));
    if (!f) jinn_persistence_fail("full-text allocation");
    if (strlen(path) >= sizeof f->path) jinn_persistence_fail("full-text path");
    strcpy(f->path, path);
    f->fp = fopen(path, "r+b");
    if (f->fp) {
        char magic[8];
        jinn_store_fread(magic, 1, 8, f->fp);
        jinn_store_fread(&f->posting_count, 8, 1, f->fp);
        if (memcmp(magic, FTS_MAGIC, 8) || f->posting_count < 0)
            jinn_persistence_fail("full-text header validation");
    } else {
        if (errno != ENOENT) jinn_persistence_fail("full-text open");
        f->fp = fopen(path, "w+bx");
        if (!f->fp) jinn_persistence_fail("full-text creation");
        jinn_store_fwrite(FTS_MAGIC, 1, 8, f->fp);
        f->posting_count = 0;
        jinn_store_fwrite(&f->posting_count, 8, 1, f->fp);
        jinn_store_finish_create(f->fp, path);
    }
    return f;
}
void jinn_fts_close(JinnFts *f) {
    if (!f) return;
    if (f->fp && fclose(f->fp) != 0) jinn_persistence_fail("full-text close");
    free(f);
}

static int64_t next_token(const char *text, int64_t len, int64_t pos, char *tok, int max_tok) {
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

static void fts_txn_rollback(void *arg) {
    JinnFts *f = (JinnFts *)arg;
    if (!f || !f->fp) return;
    jinn_store_fseek(f->fp, 8, SEEK_SET);
    jinn_store_fread(&f->posting_count, 8, 1, f->fp);
}

void jinn_fts_add(JinnFts *f, int64_t doc_id, const char *text, int64_t text_len) {
    if (!f || !f->fp || !text || text_len < 0) jinn_persistence_fail("full-text arguments");
    if (jinn_txn_active()) jinn_txn_track_aux(f->fp, fts_txn_rollback, f);
    char tok[128];
    int64_t pos = 0;
    int64_t added = 0;
    while ((pos = next_token(text, text_len, pos, tok, 128)) >= 0) {
        int32_t tlen = (int32_t)strlen(tok);
        if (tlen == 0) continue;
        if (fseek(f->fp, 0, SEEK_END) != 0 ||
            fwrite(&tlen, 4, 1, f->fp) != 1 ||
            fwrite(tok, 1, tlen, f->fp) != (size_t)tlen ||
            fwrite(&doc_id, 8, 1, f->fp) != 1) {
            jinn_persistence_fail("full-text posting append");
        }
        added++;
    }
    f->posting_count += added;
    if (fseek(f->fp, 8, SEEK_SET) != 0 ||
        fwrite(&f->posting_count, 8, 1, f->fp) != 1) {
        jinn_persistence_fail("full-text header update");
    }
    jinn_store_fflush(f->fp);
}

int64_t jinn_fts_search(JinnFts *f, const char *query, int64_t *out_ids, int64_t max_ids) {
    if (!f || !f->fp || !query) return 0;
    char lquery[128];
    int qi = 0;
    for (int i = 0; query[i] && qi < 127; i++) {
        lquery[qi++] = (char)tolower((unsigned char)query[i]);
    }
    lquery[qi] = '\0';
    int32_t qlen = (int32_t)strlen(lquery);

    jinn_store_fseek(f->fp, 16, SEEK_SET);
    int64_t found = 0;
    char tok[128];
    for (int64_t posting = 0; posting < f->posting_count; posting++) {
        int32_t tlen;
        jinn_store_fread(&tlen, 4, 1, f->fp);
        if (tlen <= 0 || tlen > 127) jinn_persistence_fail("full-text token length");
        jinn_store_fread(tok, 1, tlen, f->fp);
        tok[tlen] = '\0';
        int64_t doc_id;
        jinn_store_fread(&doc_id, 8, 1, f->fp);

        if (tlen == qlen && memcmp(tok, lquery, qlen) == 0) {
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
int64_t jinn_fts_count(JinnFts *f, const char *query) {
    return jinn_fts_search(f, query, NULL, 0);
}
int64_t jinn_fts_search_n(JinnFts *f, const char *query, int64_t qlen) {
    char buf[256];
    if (qlen > 255) qlen = 255;
    memcpy(buf, query, qlen);
    buf[qlen] = '\0';
    return jinn_fts_search(f, buf, NULL, 0);
}
int64_t jinn_fts_count_n(JinnFts *f, const char *query, int64_t qlen) {
    return jinn_fts_search_n(f, query, qlen);
}
int64_t jinn_fts_search_ids_n(JinnFts *f, const char *query, int64_t qlen,
                              int64_t *out_ids, int64_t max_ids) {
    char buf[256];
    if (qlen > 255) qlen = 255;
    memcpy(buf, query, qlen);
    buf[qlen] = '\0';
    return jinn_fts_search(f, buf, out_ids, max_ids);
}
void jinn_fts_add_n(JinnFts *f, int64_t doc_id, const char *text, int64_t text_len) {
    char buf[4096];
    if (text_len > 4095) text_len = 4095;
    memcpy(buf, text, text_len);
    buf[text_len] = '\0';
    jinn_fts_add(f, doc_id, buf, text_len);
}
int64_t jinn_fts_posting_count(JinnFts *f) {
    if (!f) return 0;
    return f->posting_count;
}
