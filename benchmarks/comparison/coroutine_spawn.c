// Coroutine spawn benchmark — C comparison using function calls
// 100K function calls, each returns a value

#include <stdio.h>

static long gen(long x) { return x; }

int main() {
    long n = 100000;
    long total = 0;
    for (long i = 0; i < n; i++) {
        total += gen(i);
    }
    printf("%ld\n", total);
    return 0;
}
