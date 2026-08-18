#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include "jinn_rt.h"
int64_t jinn_f64_format(double v, char *buf) {
    for (int prec = 15; prec <= 17; prec++) {
        int n = snprintf(buf, 32, "%.*g", prec, v);
        if (n < 0 || n >= 32) break;
        if (strtod(buf, NULL) == v) return n;
    }
    int n = snprintf(buf, 32, "%.17g", v);
    return n < 0 ? 0 : (n >= 32 ? 31 : n);
}
void *jinn_xmalloc(size_t size) {
    void *p = malloc(size);
    if (!p && size > 0) {
        fprintf(stderr, "jinn: out of memory (requested %zu bytes)\n", size);
        abort();
    }
    return p;
}
uint64_t jinn_fnv1a(const void *data, int64_t len) {
    const uint8_t *p = (const uint8_t *)data;
    uint64_t h = 14695981039346656037ULL;
    for (int64_t i = 0; i < len; i++) {
        h ^= p[i];
        h *= 1099511628211ULL;
    }
    return h;
}
void jinn_store_truncation_warn(int64_t original_len, int64_t max_len) {
    fprintf(stderr, "jinn: warning: store string truncated from %lld to %lld bytes\n",
            (long long)original_len, (long long)max_len);
}
#include <unistd.h>
#include <sys/types.h>
#include <sys/stat.h>
#define JINN_STORE_CHUNK (64 * 1024)
void jinn_store_reserve(FILE *fp, int64_t count, int64_t rec_size) {
    if (!fp || rec_size <= 0) return;
    int fd = fileno(fp);
    if (fd < 0) return;
    int64_t need = 8 + (count + 1) * rec_size;
    int64_t target = ((need + JINN_STORE_CHUNK - 1) / JINN_STORE_CHUNK) * JINN_STORE_CHUNK;
    struct stat st;
    if (fstat(fd, &st) != 0) return;
    if ((int64_t)st.st_size >= target) return;
    fflush(fp);
    (void)ftruncate(fd, target);
}
#include <string.h>
int64_t jinn_f64_to_bits(double val) {
    int64_t bits;
    memcpy(&bits, &val, sizeof(bits));
    return bits;
}
double jinn_bits_to_f64(int64_t bits) {
    double val;
    memcpy(&val, &bits, sizeof(val));
    return val;
}
const char *jinn_getenv_or_empty(const char *name) {
    const char *value = getenv(name);
    return value ? value : "";
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
void jinn_sort_i64(int64_t *data, int64_t len) {
    if (!data || len <= 1) return;
    qsort(data, (size_t)len, sizeof(int64_t), cmp_i64_asc);
}
void jinn_sort_f64(double *data, int64_t len) {
    if (!data || len <= 1) return;
    qsort(data, (size_t)len, sizeof(double), cmp_f64_asc);
}
int64_t jinn_utf8_encode(int64_t code, char *buf) {
    uint32_t c = (uint32_t)code;
    if (code < 0 || code > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF)) c = 0xFFFD;
    if (c < 0x80) {
        buf[0] = (char)c;
        return 1;
    }
    if (c < 0x800) {
        buf[0] = (char)(0xC0 | (c >> 6));
        buf[1] = (char)(0x80 | (c & 0x3F));
        return 2;
    }
    if (c < 0x10000) {
        buf[0] = (char)(0xE0 | (c >> 12));
        buf[1] = (char)(0x80 | ((c >> 6) & 0x3F));
        buf[2] = (char)(0x80 | (c & 0x3F));
        return 3;
    }
    buf[0] = (char)(0xF0 | (c >> 18));
    buf[1] = (char)(0x80 | ((c >> 12) & 0x3F));
    buf[2] = (char)(0x80 | ((c >> 6) & 0x3F));
    buf[3] = (char)(0x80 | (c & 0x3F));
    return 4;
}
int32_t jinn_ascii_imemcmp(const char *a, const char *b, int64_t n) {
    for (int64_t i = 0; i < n; i++) {
        unsigned char ca = (unsigned char)a[i];
        unsigned char cb = (unsigned char)b[i];
        if (ca >= 'A' && ca <= 'Z') ca += 32;
        if (cb >= 'A' && cb <= 'Z') cb += 32;
        if (ca != cb) return ca < cb ? -1 : 1;
    }
    return 0;
}
