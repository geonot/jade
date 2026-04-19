/*
 * runtime/util.c — Small utility functions for the Jade runtime.
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

void *jade_xmalloc(size_t size) {
    void *p = malloc(size);
    if (!p && size > 0) {
        fprintf(stderr, "jade: out of memory (requested %zu bytes)\n", size);
        abort();
    }
    return p;
}

void jade_store_truncation_warn(int64_t original_len, int64_t max_len) {
    fprintf(stderr, "jade: warning: store string truncated from %lld to %lld bytes\n",
            (long long)original_len, (long long)max_len);
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
