/* runtime/regex_helper.c — Helpers for PCRE2 ovector access */
#include <stdint.h>
#include "jade_rt.h"

int64_t jade_ovector_get(void *ovector, int64_t idx) {
    return ((int64_t *)ovector)[idx];
}
