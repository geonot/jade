/*
 * Bloom filter runtime for Jinn.
 *
 * Provides probabilistic set membership testing for fast negative lookups
 * on non-indexed fields. Uses double hashing with FNV-1a.
 *
 * File format: [8B magic "JINNBLM\0"][8B num_bits][8B num_hashes][bit array...]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "jinn_rt.h"

#define BLOOM_MAGIC "JINNBLM\0"
#define BLOOM_HEADER_SIZE 24

struct JinnBloom {
    uint8_t *bits;
    int64_t  num_bits;
    int64_t  num_hashes;
    char     path[256];
};

/* ── Hashing (uses jinn_fnv1a from util.c) ────────────────── */

static uint64_t bloom_hash(const void *data, int64_t len, int64_t k) {
    uint64_t h1 = jinn_fnv1a(data, len);
    uint64_t h2 = h1 * 0x9e3779b97f4a7c15ULL + 0x517cc1b727220a95ULL;
    return h1 + k * h2;
}

/* ── Open / Close ─────────────────────────────────────────── */

JinnBloom *jinn_bloom_create(int64_t expected_items, double fp_rate) {
    JinnBloom *b = (JinnBloom *)calloc(1, sizeof(JinnBloom));
    if (!b) return NULL;

    /* Calculate optimal size: m = -n*ln(p) / (ln2)^2 */
    double ln2 = 0.6931471805599453;
    double m = -(double)expected_items * (fp_rate < 0.001 ? -6.9 : (fp_rate < 0.01 ? -4.6 : -2.3));
    if (m < 64) m = 64;
    b->num_bits = (int64_t)m;

    /* k = (m/n) * ln2 */
    double k = ((double)b->num_bits / (double)expected_items) * ln2;
    if (k < 1) k = 1;
    if (k > 16) k = 16;
    b->num_hashes = (int64_t)k;

    int64_t bytes = (b->num_bits + 7) / 8;
    b->bits = (uint8_t *)calloc(bytes, 1);
    if (!b->bits) { free(b); return NULL; }
    b->path[0] = '\0';
    return b;
}

JinnBloom *jinn_bloom_open(const char *path, int64_t expected_items) {
    JinnBloom *b = (JinnBloom *)calloc(1, sizeof(JinnBloom));
    if (!b) return NULL;
    strncpy(b->path, path, sizeof(b->path) - 1);
    b->path[sizeof(b->path) - 1] = '\0';

    FILE *fp = fopen(path, "rb");
    if (fp) {
        char magic[8];
        fread(magic, 1, 8, fp);
        fread(&b->num_bits, 8, 1, fp);
        fread(&b->num_hashes, 8, 1, fp);
        int64_t bytes = (b->num_bits + 7) / 8;
        b->bits = (uint8_t *)calloc(bytes, 1);
        if (!b->bits) { fclose(fp); free(b); return NULL; }
        fread(b->bits, 1, bytes, fp);
        fclose(fp);
    } else {
        /* create new with defaults */
        b->num_bits = expected_items * 10;  /* ~1% FP rate */
        if (b->num_bits < 64) b->num_bits = 64;
        b->num_hashes = 7;
        int64_t bytes = (b->num_bits + 7) / 8;
        b->bits = (uint8_t *)calloc(bytes, 1);
        if (!b->bits) { free(b); return NULL; }
    }
    return b;
}

void jinn_bloom_close(JinnBloom *b) {
    if (!b) return;
    /* persist if path set */
    if (b->path[0]) {
        FILE *fp = fopen(b->path, "wb");
        if (fp) {
            fwrite(BLOOM_MAGIC, 1, 8, fp);
            fwrite(&b->num_bits, 8, 1, fp);
            fwrite(&b->num_hashes, 8, 1, fp);
            int64_t bytes = (b->num_bits + 7) / 8;
            fwrite(b->bits, 1, bytes, fp);
            fclose(fp);
        }
    }
    free(b->bits);
    free(b);
}

/* ── Insert / Query ───────────────────────────────────────── */

void jinn_bloom_add(JinnBloom *b, const void *data, int64_t len) {
    if (!b || !b->bits) return;
    for (int64_t k = 0; k < b->num_hashes; k++) {
        uint64_t h = bloom_hash(data, len, k) % (uint64_t)b->num_bits;
        b->bits[h / 8] |= (1 << (h % 8));
    }
}

/* Returns 1 if possibly present, 0 if definitely absent */
int64_t jinn_bloom_test(JinnBloom *b, const void *data, int64_t len) {
    if (!b || !b->bits) return 0;
    for (int64_t k = 0; k < b->num_hashes; k++) {
        uint64_t h = bloom_hash(data, len, k) % (uint64_t)b->num_bits;
        if (!(b->bits[h / 8] & (1 << (h % 8)))) return 0;
    }
    return 1;
}

/* ── Typed convenience wrappers ───────────────────────────── */

void jinn_bloom_add_i64(JinnBloom *b, int64_t val) {
    jinn_bloom_add(b, &val, sizeof(val));
}

int64_t jinn_bloom_test_i64(JinnBloom *b, int64_t val) {
    return jinn_bloom_test(b, &val, sizeof(val));
}

void jinn_bloom_add_str(JinnBloom *b, const char *data, int64_t len) {
    jinn_bloom_add(b, data, len);
}

int64_t jinn_bloom_test_str(JinnBloom *b, const char *data, int64_t len) {
    return jinn_bloom_test(b, data, len);
}
