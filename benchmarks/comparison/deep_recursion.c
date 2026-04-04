#include <stdio.h>
#include <stdint.h>

static int64_t deep(int64_t n, int64_t a, int64_t b, int64_t c, int64_t d) {
    if (n < 1) return a + b + c + d;
    int64_t x = a + b;
    int64_t y = c + d;
    int64_t z = x * y;
    return deep(n - 1, x, y, z, a + 1);
}

int main() {
    int64_t total = 0;
    for (int64_t i = 0; i < 100000000; i++) {
        total += deep(20, i, i + 1, i + 2, i + 3);
        total ^= i;
    }
    printf("%ld\n", total);
    return 0;
}
