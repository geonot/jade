/*
 * runtime/util.c — Small utility functions for the Jade runtime.
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

#include "jade_rt.h"
void *jade_xmalloc(size_t size) {
    void *p = malloc(size);
    if (!p && size > 0) {
        fprintf(stderr, "jade: out of memory (requested %zu bytes)\n", size);
        abort();
    }
    return p;
}

/* FNV-1a 64-bit hash for arbitrary byte sequences.
 * Single source of truth — used by index.c, bloom.c. */
uint64_t jade_fnv1a(const void *data, int64_t len) {
    const uint8_t *p = (const uint8_t *)data;
    uint64_t h = 14695981039346656037ULL;
    for (int64_t i = 0; i < len; i++) {
        h ^= p[i];
        h *= 1099511628211ULL;
    }
    return h;
}

void jade_store_truncation_warn(int64_t original_len, int64_t max_len) {
    fprintf(stderr, "jade: warning: store string truncated from %lld to %lld bytes\n",
            (long long)original_len, (long long)max_len);
}

/* R13: amortize record-store growth by extending the underlying file in
 * 64 KiB chunks rather than letting fwrite extend it record-by-record.
 * Called from compile_store_insert before each fwrite. The next-record
 * end-offset is rounded up to JADE_STORE_CHUNK; we ftruncate to that
 * size only when it would grow the file. The append fwrite that follows
 * still writes record bytes and updates the on-disk length. The chunk
 * tail beyond `count*rec_size` is allocated zero bytes which subsequent
 * inserts overwrite, eliminating per-record block-allocator hits and
 * cutting allocator churn on Linux ext4/btrfs.
 *
 * Idempotent: callers may invoke unconditionally; we no-op when the
 * file already covers the required range. fp must be a writable FILE*. */
#include <unistd.h>
#include <sys/types.h>
#include <sys/stat.h>
#define JADE_STORE_CHUNK (64 * 1024)
void jade_store_reserve(FILE *fp, int64_t count, int64_t rec_size) {
    if (!fp || rec_size <= 0) return;
    int fd = fileno(fp);
    if (fd < 0) return;
    int64_t need = 8 + (count + 1) * rec_size;
    int64_t target = ((need + JADE_STORE_CHUNK - 1) / JADE_STORE_CHUNK) * JADE_STORE_CHUNK;
    /* Use fstat so we don't disturb the FILE* stream position established
     * by the caller's fseek-to-end. */
    struct stat st;
    if (fstat(fd, &st) != 0) return;
    if ((int64_t)st.st_size >= target) return;
    fflush(fp);
    (void)ftruncate(fd, target);
}

#include <string.h>

int64_t jade_f64_to_bits(double val) {
    int64_t bits;
    memcpy(&bits, &val, sizeof(bits));
    return bits;
}

double jade_bits_to_f64(int64_t bits) {
    double val;
    memcpy(&val, &bits, sizeof(val));
    return val;
}

static int cmp_i64_asc(const void *a, const void *b) {
    int64_t lhs = *(const int64_t *)a;
    int64_t rhs = *(const int64_t *)b;
    if (lhs < rhs) return -1;
    if (lhs > rhs) return 1;
    return 0;
}

static int cmp_f64_asc(const void *a, const void *b) {
    double lhs = *(const double *)a;
    double rhs = *(const double *)b;
    if (lhs < rhs) return -1;
    if (lhs > rhs) return 1;
    return 0;
}

void jade_sort_i64(int64_t *data, int64_t len) {
    if (!data || len <= 1) return;
    qsort(data, (size_t)len, sizeof(int64_t), cmp_i64_asc);
}

void jade_sort_f64(double *data, int64_t len) {
    if (!data || len <= 1) return;
    qsort(data, (size_t)len, sizeof(double), cmp_f64_asc);
}
