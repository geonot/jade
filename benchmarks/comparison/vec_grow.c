#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

int main() {
    int64_t cap = 16;
    int64_t len = 0;
    int64_t *buf = (int64_t *)malloc(cap * sizeof(int64_t));

    for (int64_t i = 0; i < 400000000; i++) {
        if (len >= cap) {
            cap *= 2;
            buf = (int64_t *)realloc(buf, cap * sizeof(int64_t));
        }
        buf[len++] = i;
    }

    for (int64_t i = 0; i < len; i++) {
        buf[i] = buf[i] ^ (i + 1);
    }

    int64_t total = 0;
    for (int64_t i = 0; i < len; i++) {
        total += buf[i];
    }
    printf("%ld\n", total);
    free(buf);
    return 0;
}
