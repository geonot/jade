/*
 * Column store runtime for Jinn.
 *
 * Per-field column files enable vectorized aggregation (sum/avg/min/max)
 * on contiguous typed arrays without deserializing full records.
 *
 * File format: [8B magic "JADECOL\0"][8B count][8B elem_size][column data...]
 * Each element is elem_size bytes, stored contiguously.
 *
 * For integer columns (i64), SIMD-accelerated aggregation is available
 * when compiled with AVX2/NEON support.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "jinn_rt.h"

#define COL_MAGIC "JADECOL\0"
#define COL_HEADER_SIZE 24

/* Safe multiplication with overflow check.  Returns 0 on overflow. */
static inline size_t safe_mul(int64_t a, int64_t b) {
    if (a <= 0 || b <= 0) return 0;
    if (a > (int64_t)(SIZE_MAX / (uint64_t)b)) return 0;  /* overflow */
    return (size_t)a * (size_t)b;
}

/* Allocate a buffer for count elements of elem_bytes each.
 * Returns NULL (not crashing) on overflow or OOM.                    */
static inline void *col_alloc(int64_t count, int64_t elem_bytes) {
    size_t bytes = safe_mul(count, elem_bytes);
    if (bytes == 0) return NULL;
    return malloc(bytes);
}

struct JinnCol {
    FILE   *fp;
    int64_t count;
    int64_t elem_size;  /* bytes per element */
    char    path[256];
};

/* ── Open / Close ─────────────────────────────────────────── */

JinnCol *jinn_col_open(const char *path, int64_t elem_size) {
    JinnCol *c = (JinnCol *)calloc(1, sizeof(JinnCol));
    if (!c) return NULL;
    strncpy(c->path, path, 255);
    c->elem_size = elem_size;

    c->fp = fopen(path, "r+b");
    if (c->fp) {
        /* existing file — read header */
        char magic[8];
        fread(magic, 1, 8, c->fp);
        fread(&c->count, 8, 1, c->fp);
        fread(&c->elem_size, 8, 1, c->fp);  /* override with stored size */
    } else {
        /* create new */
        c->fp = fopen(path, "w+b");
        if (!c->fp) { free(c); return NULL; }
        fwrite(COL_MAGIC, 1, 8, c->fp);
        c->count = 0;
        fwrite(&c->count, 8, 1, c->fp);
        fwrite(&c->elem_size, 8, 1, c->fp);
        fflush(c->fp);
    }
    return c;
}

void jinn_col_close(JinnCol *c) {
    if (!c) return;
    if (c->fp) fclose(c->fp);
    free(c);
}

/* ── Append / Count ───────────────────────────────────────── */

void jinn_col_append(JinnCol *c, const void *data) {
    if (!c || !c->fp) return;
    /* seek to end of data */
    if (fseek(c->fp, COL_HEADER_SIZE + c->count * c->elem_size, SEEK_SET) != 0) {
        fprintf(stderr, "jinn: column: fseek to data region failed\n");
        return;
    }
    if (fwrite(data, c->elem_size, 1, c->fp) != 1) {
        fprintf(stderr, "jinn: column: fwrite data failed\n");
        return;
    }
    c->count++;
    /* update count in header */
    if (fseek(c->fp, 8, SEEK_SET) != 0 ||
        fwrite(&c->count, 8, 1, c->fp) != 1) {
        fprintf(stderr, "jinn: column: failed to update count header\n");
        c->count--; /* revert in-memory state */
    }
    fflush(c->fp);
}

int64_t jinn_col_count(JinnCol *c) {
    return c ? c->count : 0;
}

/* ── Bulk read into buffer ────────────────────────────────── */

/* Read all column values into a caller-supplied buffer.
 * Returns number of elements read. */
int64_t jinn_col_read_all(JinnCol *c, void *buf, int64_t max_elems) {
    if (!c || !c->fp || c->count == 0) return 0;
    int64_t n = c->count < max_elems ? c->count : max_elems;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t got = (int64_t)fread(buf, c->elem_size, n, c->fp);
    return got;
}

/* ── Vectorized i64 aggregation (scalar fallback) ─────────── */

int64_t jinn_col_sum_i64(JinnCol *c) {
    if (!c || c->count == 0) return 0;
    int64_t *buf = (int64_t *)col_alloc(c->count, 8);
    if (!buf) return 0;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t n = (int64_t)fread(buf, 8, c->count, c->fp);
    int64_t sum = 0;

    /* Compiler auto-vectorization-friendly loop */
    for (int64_t i = 0; i < n; i++) {
        sum += buf[i];
    }
    free(buf);
    return sum;
}

int64_t jinn_col_min_i64(JinnCol *c) {
    if (!c || c->count == 0) return 0;
    int64_t *buf = (int64_t *)col_alloc(c->count, 8);
    if (!buf) return 0;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t n = (int64_t)fread(buf, 8, c->count, c->fp);
    int64_t val = buf[0];
    for (int64_t i = 1; i < n; i++) {
        if (buf[i] < val) val = buf[i];
    }
    free(buf);
    return val;
}

int64_t jinn_col_max_i64(JinnCol *c) {
    if (!c || c->count == 0) return 0;
    int64_t *buf = (int64_t *)col_alloc(c->count, 8);
    if (!buf) return 0;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t n = (int64_t)fread(buf, 8, c->count, c->fp);
    int64_t val = buf[0];
    for (int64_t i = 1; i < n; i++) {
        if (buf[i] > val) val = buf[i];
    }
    free(buf);
    return val;
}

/* avg returns the sum — caller divides by count for f64 result */
int64_t jinn_col_avg_sum_i64(JinnCol *c) {
    return jinn_col_sum_i64(c);
}

/* ── Vectorized f64 aggregation ───────────────────────────── */

double jinn_col_sum_f64(JinnCol *c) {
    if (!c || c->count == 0) return 0.0;
    double *buf = (double *)col_alloc(c->count, 8);
    if (!buf) return 0.0;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t n = (int64_t)fread(buf, 8, c->count, c->fp);
    double sum = 0.0;
    for (int64_t i = 0; i < n; i++) {
        sum += buf[i];
    }
    free(buf);
    return sum;
}

double jinn_col_min_f64(JinnCol *c) {
    if (!c || c->count == 0) return 0.0;
    double *buf = (double *)col_alloc(c->count, 8);
    if (!buf) return 0.0;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t n = (int64_t)fread(buf, 8, c->count, c->fp);
    double val = buf[0];
    for (int64_t i = 1; i < n; i++) {
        if (buf[i] < val) val = buf[i];
    }
    free(buf);
    return val;
}

double jinn_col_max_f64(JinnCol *c) {
    if (!c || c->count == 0) return 0.0;
    double *buf = (double *)col_alloc(c->count, 8);
    if (!buf) return 0.0;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t n = (int64_t)fread(buf, 8, c->count, c->fp);
    double val = buf[0];
    for (int64_t i = 1; i < n; i++) {
        if (buf[i] > val) val = buf[i];
    }
    free(buf);
    return val;
}

/* ── Distinct count (i64) — hash-based ────────────────────── */

int64_t jinn_col_distinct_i64(JinnCol *c) {
    if (!c || c->count == 0) return 0;
    int64_t *buf = (int64_t *)col_alloc(c->count, 8);
    if (!buf) return 0;
    fseek(c->fp, COL_HEADER_SIZE, SEEK_SET);
    int64_t n = (int64_t)fread(buf, 8, c->count, c->fp);

    /* Hash-set based distinct count — O(n) on average.
     * Table size is 2× data size for low load factor; falls back
     * to brute-force O(n²) only if the table allocation fails.    */
    int64_t unique = 0;
    int64_t tbl_cap = n < 16 ? 32 : n * 2;
    /* Ensure power-of-2 capacity */
    int64_t v = tbl_cap - 1;
    v |= v >> 1; v |= v >> 2; v |= v >> 4;
    v |= v >> 8; v |= v >> 16; v |= v >> 32;
    tbl_cap = v + 1;

    size_t tbl_bytes = safe_mul(tbl_cap, (int64_t)sizeof(int64_t));
    size_t flag_bytes = safe_mul(tbl_cap, 1);
    uint8_t *flags = NULL;
    int64_t *tbl = NULL;
    int use_hash = 0;

    if (tbl_bytes > 0 && flag_bytes > 0) {
        tbl = (int64_t *)malloc(tbl_bytes);
        flags = (uint8_t *)calloc((size_t)tbl_cap, 1);
        if (tbl && flags) use_hash = 1;
    }

    if (use_hash) {
        uint64_t mask = (uint64_t)(tbl_cap - 1);
        for (int64_t i = 0; i < n; i++) {
            uint64_t h = (uint64_t)buf[i] * 0x9E3779B97F4A7C15ULL;
            uint64_t slot = h & mask;
            for (;;) {
                if (!flags[slot]) {
                    flags[slot] = 1;
                    tbl[slot] = buf[i];
                    unique++;
                    break;
                }
                if (tbl[slot] == buf[i]) break; /* duplicate */
                slot = (slot + 1) & mask;
            }
        }
    } else {
        /* Fallback: brute-force for very large or OOM scenarios */
        for (int64_t i = 0; i < n; i++) {
            int found = 0;
            for (int64_t j = 0; j < i; j++) {
                if (buf[j] == buf[i]) { found = 1; break; }
            }
            if (!found) unique++;
        }
    }

    free(tbl);
    free(flags);
    free(buf);
    return unique;
}
