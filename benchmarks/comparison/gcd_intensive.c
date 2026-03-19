#include <stdio.h>
#include <stdint.h>

static int64_t gcd(int64_t a, int64_t b) {
    while (b != 0) {
        int64_t t = b;
        b = a % b;
        a = t;
    }
    return a;
}

int main(void) {
    int64_t sum = 0;
    for (int64_t i = 1; i < 10000; i++) {
        for (int64_t j = i + 1; j < 10000; j += 100) {
            sum += gcd(i, j);
        }
    }
    printf("%ld\n", sum);
    return 0;
}
