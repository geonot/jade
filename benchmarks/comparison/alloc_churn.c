#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

static int64_t churn(int64_t n) {
    int64_t *buf = (int64_t *)malloc(10 * sizeof(int64_t));
    for (int j = 0; j < 10; j++) buf[j] = n + j;
    int64_t s = buf[n % 10];
    free(buf);
    return s;
}

int main() {
    int64_t total = 0;
    for (int64_t i = 0; i < 50000000; i++) {
        total += churn(i);
        total ^= i;
    }
    printf("%ld\n", total);
    return 0;
}
