#include <stdio.h>
#include <stdint.h>

static int64_t collatz_steps(int64_t n) {
    int64_t steps = 0;
    while (n != 1) {
        if (n % 2 == 0)
            n = n / 2;
        else
            n = 3 * n + 1;
        steps++;
    }
    return steps;
}

int main(void) {
    int64_t max_steps = 0, max_n = 0;
    for (int64_t n = 1; n < 5000000; n++) {
        int64_t s = collatz_steps(n);
        if (s > max_steps) {
            max_steps = s;
            max_n = n;
        }
    }
    printf("%ld\n%ld\n", max_n, max_steps);
    return 0;
}
