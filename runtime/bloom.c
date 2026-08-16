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

static uint64_t bloom_hash(const void *data, int64_t len, int64_t k) {
    uint64_t h1 = jinn_fnv1a(data, len);
    uint64_t h2 = h1 * 0x9e3779b97f4a7c15ULL + 0x517cc1b727220a95ULL;
    return h1 + k * h2;
}

JinnBloom *jinn_bloom_create(int64_t expected_items, double fp_rate) {
    JinnBloom *b = (JinnBloom *)calloc(1, sizeof(JinnBloom));
    if (!b) return NULL;

    double ln2 = 0.6931471805599453;
    double m = -(double)expected_items * (fp_rate < 0.001 ? -6.9 : (fp_rate < 0.01 ? -4.6 : -2.3));
    if (m < 64) m = 64;
    b->num_bits = (int64_t)m;

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
        b->num_bits = expected_items * 10;
        if (b->num_bits < 64) b->num_bits = 64;
        b->num_hashes = 7;
        int64_t bytes = (b->num_bits + 7) / 8;
        b->bits = (uint8_t *)calloc(bytes, 1);
        if (!b->bits) { free(b); return NULL; }
    }
    return b;
}
static int bloom_fill(FILE *tmp, void *arg) {
    JinnBloom *b = (JinnBloom *)arg;
    int64_t bytes = (b->num_bits + 7) / 8;
    if (fwrite(BLOOM_MAGIC, 1, 8, tmp) != 8 ||
        fwrite(&b->num_bits, 8, 1, tmp) != 1 ||
        fwrite(&b->num_hashes, 8, 1, tmp) != 1 ||
        fwrite(b->bits, 1, (size_t)bytes, tmp) != (size_t)bytes) {
        return -1;
    }
    return 0;
}
void jinn_bloom_close(JinnBloom *b) {
    if (!b) return;
    if (b->path[0]) {
        (void)jinn_atomic_rewrite(b->path, bloom_fill, b);
    }
    free(b->bits);
    free(b);
}
typedef struct {
    JinnBloom *b;
    uint8_t   *bits;
    int64_t    num_bits;
} BloomTxnSnap;
static void bloom_txn_rollback(void *arg) {
    BloomTxnSnap *s = (BloomTxnSnap *)arg;
    if (s->b->num_bits == s->num_bits) {
        memcpy(s->b->bits, s->bits, (size_t)((s->num_bits + 7) / 8));
    }
}
static void bloom_txn_release(void *arg) {
    BloomTxnSnap *s = (BloomTxnSnap *)arg;
    free(s->bits);
    free(s);
}
static void bloom_txn_guard(JinnBloom *b) {
    if (!jinn_txn_active() || jinn_txn_is_tracked(b)) return;
    int64_t bytes = (b->num_bits + 7) / 8;
    BloomTxnSnap *s = (BloomTxnSnap *)malloc(sizeof *s);
    uint8_t *copy = (uint8_t *)malloc((size_t)bytes);
    if (!s || !copy) {
        fprintf(stderr, "jinn: bloom: out of memory snapshotting for a "
                        "transaction — aborting (cannot guarantee rollback)\n");
        abort();
    }
    memcpy(copy, b->bits, (size_t)bytes);
    s->b = b;
    s->bits = copy;
    s->num_bits = b->num_bits;
    jinn_txn_track_mem(b, bloom_txn_rollback, bloom_txn_release, s);
}
void jinn_bloom_add(JinnBloom *b, const void *data, int64_t len) {
    if (!b || !b->bits) return;
    bloom_txn_guard(b);
    for (int64_t k = 0; k < b->num_hashes; k++) {
        uint64_t h = bloom_hash(data, len, k) % (uint64_t)b->num_bits;
        b->bits[h / 8] |= (1 << (h % 8));
    }
}
int64_t jinn_bloom_test(JinnBloom *b, const void *data, int64_t len) {
    if (!b || !b->bits) return 0;
    for (int64_t k = 0; k < b->num_hashes; k++) {
        uint64_t h = bloom_hash(data, len, k) % (uint64_t)b->num_bits;
        if (!(b->bits[h / 8] & (1 << (h % 8)))) return 0;
    }
    return 1;
}
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
