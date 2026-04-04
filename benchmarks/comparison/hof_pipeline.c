#include <stdio.h>
#include <stdint.h>

typedef int64_t (*fn_i64)(int64_t);

static int64_t double_val(int64_t x) { return x * 2; }
static int64_t add_one(int64_t x) { return x + 1; }

static int64_t apply(fn_i64 f, int64_t x) { return f(x); }

int main(void) {
    int64_t total = 0;
    for (int64_t i = 0; i < 2000000000; i++) {
        total += apply(double_val, i);
        total += add_one(double_val(i));
        total ^= i;
    }
    printf("%ld\n", total);
    return 0;
}
