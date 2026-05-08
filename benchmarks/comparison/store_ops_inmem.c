/* R7/B-STORE — in-memory baseline matching benchmarks/store_ops_inmem.jn.
 * Uses parallel int64 arrays + linear search to mirror the Jinn Vec form
 * (no hash table). The original store_ops.c kept a Record[] struct array;
 * this variant aligns the data layout with the Jinn side. */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

int main(void) {
    int64_t cap = 10000;
    int64_t *keys = malloc((size_t)cap * sizeof(int64_t));
    int64_t *vals = malloc((size_t)cap * sizeof(int64_t));
    int64_t n = 0;
    for (int64_t i = 0; i < 10000; i++) {
        keys[n] = i;
        vals[n] = i * 7;
        n++;
    }

    int64_t total = 0;
    for (int64_t j = 0; j < 1000; j++) {
        int64_t idx = -1;
        for (int64_t k = 0; k < n; k++) {
            if (keys[k] == j) { idx = k; break; }
        }
        if (idx >= 0) total += vals[idx];
    }
    printf("%lld\n", (long long)total);
    printf("%lld\n", (long long)n);
    free(keys);
    free(vals);
    return 0;
}
