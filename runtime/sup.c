#include "jinn_rt.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#ifndef JINN_SUP_MAX_RESTARTS
#define JINN_SUP_MAX_RESTARTS 16
#endif
typedef struct {
    jinn_sup_factory_t  factory;
    jinn_sup_loop_t     loop_fn;
    void               *mb_ptr;
    const char         *name;
    int                 alive;
} jinn_sup_child_slot_t;
struct jinn_sup {
    jinn_sup_strategy_t   strategy;
    jinn_sup_child_slot_t *children;
    size_t                 n_children;
    size_t                 cap_children;
    int                    restart_count;
    int                    started;
    pthread_mutex_t        lock;
};
typedef struct {
    jinn_sup_t *sup;
    size_t      idx;
} jinn_sup_child_arg_t;
static void jinn_sup_spawn_one(jinn_sup_t *sup, size_t idx);
static void sup_on_child_exit(void *arg);
jinn_sup_t *jinn_sup_create(jinn_sup_strategy_t strategy) {
    jinn_sup_t *s = (jinn_sup_t *)calloc(1, sizeof(*s));
    if (!s) return NULL;
    s->strategy = strategy;
    s->cap_children = 4;
    s->children = (jinn_sup_child_slot_t *)calloc(s->cap_children, sizeof(*s->children));
    if (!s->children) { free(s); return NULL; }
    pthread_mutex_init(&s->lock, NULL);
    return s;
}
size_t jinn_sup_register(jinn_sup_t *sup, jinn_sup_factory_t factory,
                         jinn_sup_loop_t loop_fn, const char *name) {
    if (!sup || !factory || !loop_fn) return (size_t)-1;
    if (sup->n_children == sup->cap_children) {
        size_t nc = sup->cap_children * 2;
        jinn_sup_child_slot_t *nb = (jinn_sup_child_slot_t *)realloc(
            sup->children, nc * sizeof(*sup->children));
        if (!nb) return (size_t)-1;
        memset(nb + sup->cap_children, 0,
               (nc - sup->cap_children) * sizeof(*sup->children));
        sup->children = nb;
        sup->cap_children = nc;
    }
    size_t idx = sup->n_children++;
    sup->children[idx].factory = factory;
    sup->children[idx].loop_fn = loop_fn;
    sup->children[idx].name    = name;
    sup->children[idx].mb_ptr  = NULL;
    sup->children[idx].alive   = 0;
    return idx;
}
static void sup_child_entry(void *arg) {
    jinn_sup_child_arg_t *a = (jinn_sup_child_arg_t *)arg;
    jinn_sup_t *sup = a->sup;
    size_t idx = a->idx;
    if (idx >= sup->n_children) return;
    jinn_sup_child_slot_t *slot = &sup->children[idx];
    void *mb = slot->mb_ptr;
    jinn_sup_loop_t lf = slot->loop_fn;
    if (mb && lf) {
        lf(mb);
    }
}
static void jinn_sup_spawn_one(jinn_sup_t *sup, size_t idx) {
    if (!sup || idx >= sup->n_children) return;
    jinn_sup_child_slot_t *slot = &sup->children[idx];
    if (slot->mb_ptr) {
        jinn_actor_destroy(slot->mb_ptr);
        slot->mb_ptr = NULL;
    }
    slot->mb_ptr = slot->factory();
    if (!slot->mb_ptr) {
        slot->alive = 0;
        return;
    }
    slot->alive = 1;
    jinn_sup_child_arg_t *carg = (jinn_sup_child_arg_t *)calloc(1, sizeof(*carg));
    if (!carg) return;
    carg->sup = sup;
    carg->idx = idx;
    jinn_coro_t *coro = jinn_coro_create(sup_child_entry, carg);
    if (!coro) { free(carg); return; }
    jinn_coro_set_daemon(coro);
    jinn_coro_set_on_exit(coro, sup_on_child_exit, carg);
    jinn_sched_spawn(coro);
}
static void sup_on_child_exit(void *arg) {
    jinn_sup_child_arg_t *a = (jinn_sup_child_arg_t *)arg;
    if (!a) return;
    jinn_sup_t *sup = a->sup;
    size_t idx = a->idx;
    free(a);
    if (!sup || idx >= sup->n_children) return;
    pthread_mutex_lock(&sup->lock);
    jinn_sup_child_slot_t *slot = &sup->children[idx];
    slot->alive = 0;
    if (slot->mb_ptr) {
        jinn_actor_destroy(slot->mb_ptr);
        slot->mb_ptr = NULL;
    }
    if (sup->restart_count >= JINN_SUP_MAX_RESTARTS) {
        fprintf(stderr,
                "jinn: supervisor: child '%s' exceeded the restart cap (%d) — "
                "no longer supervising it\n",
                slot->name ? slot->name : "?", JINN_SUP_MAX_RESTARTS);
        pthread_mutex_unlock(&sup->lock);
        return;
    }
    sup->restart_count++;
    switch (sup->strategy) {
    case JINN_SUP_ONE_FOR_ONE:
        jinn_sup_spawn_one(sup, idx);
        break;
    case JINN_SUP_ONE_FOR_ALL:
        jinn_sup_spawn_one(sup, idx);
        for (size_t i = 0; i < sup->n_children; i++) {
            if (i == idx) continue;
            if (sup->children[i].alive && sup->children[i].mb_ptr) {
                jinn_actor_stop(sup->children[i].mb_ptr);
            }
        }
        break;
    case JINN_SUP_REST_FOR_ONE:
        jinn_sup_spawn_one(sup, idx);
        for (size_t i = idx + 1; i < sup->n_children; i++) {
            if (sup->children[i].alive && sup->children[i].mb_ptr) {
                jinn_actor_stop(sup->children[i].mb_ptr);
            }
        }
        break;
    }
    pthread_mutex_unlock(&sup->lock);
}
void jinn_sup_start(jinn_sup_t *sup) {
    if (!sup) return;
    pthread_mutex_lock(&sup->lock);
    if (sup->started) {
        pthread_mutex_unlock(&sup->lock);
        return;
    }
    sup->started = 1;
    for (size_t i = 0; i < sup->n_children; i++) {
        jinn_sup_spawn_one(sup, i);
    }
    pthread_mutex_unlock(&sup->lock);
}
int jinn_sup_restart_count(jinn_sup_t *sup) {
    return sup ? sup->restart_count : 0;
}
void *jinn_sup_child_mailbox(jinn_sup_t *sup, size_t idx) {
    if (!sup || idx >= sup->n_children) return NULL;
    return sup->children[idx].mb_ptr;
}
void jinn_sup_destroy(jinn_sup_t *sup) {
    if (!sup) return;
    for (size_t i = 0; i < sup->n_children; i++) {
        if (sup->children[i].mb_ptr) {
            jinn_actor_destroy(sup->children[i].mb_ptr);
        }
    }
    free(sup->children);
    pthread_mutex_destroy(&sup->lock);
    free(sup);
}
