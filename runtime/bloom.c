/*
 * Bloom filter runtime for Jade.
 *
 * Provides probabilistic set membership testing for fast negative lookups
 * on non-indexed fields. Uses double hashing with FNV-1a.
 *
 * File format: [8B magic "JADEBLM\0"][8B num_bits][8B num_hashes][bit array...]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#define BLOOM_MAGIC "JADEBLM\0"
#define BLOOM_HEADER_SIZE 24

typedef struct {
    uint8_t *bits;
    int64_t  num_bits;
    int64_t  num_hashes;
    char     path[256];
} JadeBloom;

/* ── FNV-1a hashing ───────────────────────────────────────── */

static uint64_t fnv1a(const void *data, int64_t len) {
    uint64_t h = 0xcbf29ce484222325ULL;
    const uint8_t *p = (const uint8_t *)data;
    for (int64_t i = 0; i < len; i++) {
        h ^= p[i];
        h *= 0x100000001b3ULL;
    }
    return h;
}

static uint64_t bloom_hash(const void *data, int64_t len, int64_t k) {
    uint64_t h1 = fnv1a(data, len);
    uint64_t h2 = h1 * 0x9e3779b97f4a7c15ULL + 0x517cc1b727220a95ULL;
    return h1 + k * h2;
}

/* ── Open / Close ─────────────────────────────────────────── */

JadeBloom *jade_bloom_create(int64_t expected_items, double fp_rate) {
    JadeBloom *b = (JadeBloom *)calloc(1, sizeof(JadeBloom));
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

JadeBloom *jade_bloom_open(const char *path, int64_t expected_items) {
    JadeBloom *b = (JadeBloom *)calloc(1, sizeof(JadeBloom));
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

void jade_bloom_close(JadeBloom *b) {
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

void jade_bloom_add(JadeBloom *b, const void *data, int64_t len) {
    if (!b || !b->bits) return;
    for (int64_t k = 0; k < b->num_hashes; k++) {
        uint64_t h = bloom_hash(data, len, k) % (uint64_t)b->num_bits;
        b->bits[h / 8] |= (1 << (h % 8));
    }
}

/* Returns 1 if possibly present, 0 if definitely absent */
int64_t jade_bloom_test(JadeBloom *b, const void *data, int64_t len) {
    if (!b || !b->bits) return 0;
    for (int64_t k = 0; k < b->num_hashes; k++) {
        uint64_t h = bloom_hash(data, len, k) % (uint64_t)b->num_bits;
        if (!(b->bits[h / 8] & (1 << (h % 8)))) return 0;
    }
    return 1;
}

/* ── Typed convenience wrappers ───────────────────────────── */

void jade_bloom_add_i64(JadeBloom *b, int64_t val) {
    jade_bloom_add(b, &val, sizeof(val));
}

int64_t jade_bloom_test_i64(JadeBloom *b, int64_t val) {
    return jade_bloom_test(b, &val, sizeof(val));
}

void jade_bloom_add_str(JadeBloom *b, const char *data, int64_t len) {
    jade_bloom_add(b, data, len);
}

int64_t jade_bloom_test_str(JadeBloom *b, const char *data, int64_t len) {
    return jade_bloom_test(b, data, len);
}
