/*
 * Jade Pool Allocator — Fixed-size slab allocator for hot-path objects.
 *
 * O(1) alloc and free via an intrusive freelist.
 * Page-aligned backing store for cache friendliness.
 */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>

typedef struct jade_pool {
    void    *backing;       /* raw backing memory */
    void    *freelist;      /* head of intrusive freelist */
    size_t   obj_size;      /* bytes per object (>= sizeof(void*)) */
    size_t   capacity;      /* total slots */
    size_t   allocated;     /* currently in use */
} jade_pool_t;

jade_pool_t *jade_pool_create(size_t obj_size, size_t count) {
    /* Ensure objects are large enough to hold a freelist pointer */
    if (obj_size < sizeof(void *))
        obj_size = sizeof(void *);
    /* Align obj_size to 8-byte boundary */
    obj_size = (obj_size + 7) & ~(size_t)7;

    jade_pool_t *pool = (jade_pool_t *)malloc(sizeof(jade_pool_t));
    if (!pool) return NULL;

    size_t total = obj_size * count;
    void *backing = NULL;
    /* Try page-aligned allocation for cache friendliness */
    if (posix_memalign(&backing, 4096, total) != 0) {
        backing = malloc(total);
        if (!backing) { free(pool); return NULL; }
    }
    memset(backing, 0, total);

    pool->backing   = backing;
    pool->obj_size  = obj_size;
    pool->capacity  = count;
    pool->allocated = 0;

    /* Build intrusive freelist: each free slot stores a pointer to the next */
    pool->freelist = NULL;
    char *base = (char *)backing;
    for (size_t i = count; i > 0; i--) {
        void *slot = base + (i - 1) * obj_size;
        *(void **)slot = pool->freelist;
        pool->freelist = slot;
    }

    return pool;
}

void *jade_pool_alloc(jade_pool_t *pool) {
    if (!pool || !pool->freelist)
        return NULL;
    void *slot = pool->freelist;
    pool->freelist = *(void **)slot;
    pool->allocated++;
    return slot;
}

void jade_pool_free(jade_pool_t *pool, void *ptr) {
    if (!pool || !ptr) return;

    /* Validate ptr is within the pool's backing store */
    char *base = (char *)pool->backing;
    char *p    = (char *)ptr;
    size_t offset = (size_t)(p - base);
    if (p < base || offset >= pool->obj_size * pool->capacity)
        return; /* ptr not from this pool — ignore */
    if (offset % pool->obj_size != 0)
        return; /* misaligned — ignore */

    *(void **)ptr = pool->freelist;
    pool->freelist = ptr;
    pool->allocated--;
}

void jade_pool_reset(jade_pool_t *pool) {
    if (!pool) return;
    pool->allocated = 0;
    /* Rebuild freelist */
    pool->freelist = NULL;
    char *base = (char *)pool->backing;
    for (size_t i = pool->capacity; i > 0; i--) {
        void *slot = base + (i - 1) * pool->obj_size;
        *(void **)slot = pool->freelist;
        pool->freelist = slot;
    }
}

void jade_pool_destroy(jade_pool_t *pool) {
    if (!pool) return;
    free(pool->backing);
    free(pool);
}

size_t jade_pool_count(jade_pool_t *pool) {
    return pool ? pool->allocated : 0;
}

size_t jade_pool_capacity(jade_pool_t *pool) {
    return pool ? pool->capacity : 0;
}
