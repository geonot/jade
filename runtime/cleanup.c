#include "jinn_rt.h"
#include <string.h>

typedef struct cleanup_node {
    struct cleanup_node *next;
    void (*run)(void *);
    void (*drop)(void *);
    void *data;
} cleanup_node;

typedef struct cleanup_frame {
    cleanup_node *actions;
    cleanup_node *owners;
} cleanup_frame;

static void *cleanup_alloc(size_t size) {
    void *p = calloc(1, size);
    if (!p) {
        fputs("jinn: cleanup allocation failed\n", stderr);
        abort();
    }
    return p;
}

void *jinn_cleanup_new(void) {
    return cleanup_alloc(sizeof(cleanup_frame));
}

void *jinn_cleanup_cell(void *frame, uint64_t size, void (*drop)(void *)) {
    cleanup_frame *f = frame;
    cleanup_node *n = cleanup_alloc(sizeof(cleanup_node));
    n->data = cleanup_alloc(size ? size : 1);
    n->drop = drop;
    n->next = f->owners;
    f->owners = n;
    return n->data;
}

void jinn_cleanup_push(void *frame, void (*run)(void *), void *env) {
    cleanup_frame *f = frame;
    cleanup_node *n = cleanup_alloc(sizeof(cleanup_node));
    n->run = run;
    n->data = env;
    if (env) memcpy(&n->drop, env, sizeof(n->drop));
    n->next = f->actions;
    f->actions = n;
}

void jinn_cleanup_run(void *frame) {
    cleanup_frame *f = frame;
    jinn_coro_t *c = jinn_current_coro();
    if (c) c->cleanup_depth++;
    while (f->actions) {
        cleanup_node *n = f->actions;
        f->actions = n->next;
        n->run(n->data);
        if (n->drop) n->drop(n->data);
        free(n);
    }
    while (f->owners) {
        cleanup_node *n = f->owners;
        f->owners = n->next;
        if (n->drop) n->drop(n->data);
        free(n->data);
        free(n);
    }
    free(f);
    if (c) c->cleanup_depth--;
}
