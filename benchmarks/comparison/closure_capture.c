#include <stdio.h>
#include <stdint.h>

typedef int64_t (*fn_i64)(int64_t);

static int64_t base_val = 100;

static int64_t adder(int64_t x) {
    return base_val + x;
}

static int64_t apply(fn_i64 f, int64_t x) {
    return f(x);
}

int main(void) {
    int64_t total = 0;
    for (int64_t i = 0; i < 10000000; i++) {
        total += adder(i);
        total += apply(adder, i);
    }
    printf("%ld\n", total);
    return 0;
}
