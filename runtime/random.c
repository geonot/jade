/* runtime/random.c — Always-linked stubs used by std/random.jn,
 * std/time.jn, std/uuid.jn. These avoid an OpenSSL dependency.
 *
 * `__random_u64` prefers `getrandom(2)` (Linux) / `arc4random_buf(3)` (BSD,
 * macOS); falls back to `/dev/urandom`, then to a time + ASLR-mixed seed.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <math.h>

#if defined(__linux__)
#  include <sys/random.h>
#endif

long long __random_u64(void) {
    unsigned long long v = 0;
#if defined(__linux__)
    if (getrandom(&v, sizeof v, 0) == (ssize_t)sizeof v) return (long long)v;
#endif
    FILE *f = fopen("/dev/urandom", "rb");
    if (f) {
        size_t got = fread(&v, 1, sizeof v, f);
        fclose(f);
        if (got == sizeof v) return (long long)v;
    }
    /* Fallback: mix wall time, monotonic time, pid, and a stack address. */
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    v ^= (unsigned long long)ts.tv_sec * 0x9E3779B97F4A7C15ULL;
    v ^= (unsigned long long)ts.tv_nsec;
    v ^= (unsigned long long)getpid() << 17;
    v ^= (unsigned long long)(uintptr_t)&ts;
    return (long long)v;
}

/* Thin libm aliases used by std/random.jn (Box-Muller etc.). */
double __sqrt(double x) { return sqrt(x); }
double __ln(double x)   { return log(x); }
double __cos(double x)  { return cos(x); }

/* Monotonic time in nanoseconds. Used by std/random.jn for default seeding,
 * std/time.jn for Instant/Duration, and std/uuid.jn for v7 timestamps. */
long long __time_monotonic(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) return 0;
    return (long long)ts.tv_sec * 1000000000LL + (long long)ts.tv_nsec;
}
