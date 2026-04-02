/* Dispatch yield benchmark — C comparison
 * Simulates generator yielding 1M values via function calls.
 */
#include <stdio.h>

int main(void) {
    long total = 0;
    for (long i = 0; i < 1000000; i++) {
        total += i;
    }
    printf("%ld\n", total);
    return 0;
}
