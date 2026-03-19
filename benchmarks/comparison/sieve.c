#include <stdio.h>
#include <stdint.h>

static int is_prime(int64_t n) {
    if (n < 2) return 0;
    for (int64_t i = 2; i * i <= n; i++) {
        if (n % i == 0) return 0;
    }
    return 1;
}

int main(void) {
    int64_t count = 0;
    for (int64_t n = 2; n < 1000000; n++) {
        if (is_prime(n)) count++;
    }
    printf("%ld\n", count);
    return 0;
}
