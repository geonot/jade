/*
 * Jinn Runtime — Supervisor (OTP-style).
 *
 * Manages a fixed set of child actors. When a child's coroutine exits
 * (via on_exit_cb), the supervisor restarts it according to its strategy:
 *
 *   OneForOne   — restart only the failed child.
 *   OneForAll   — restart all children when any one exits.
 *   RestForOne  — restart the failed child and every child registered after.
 *
 * Restart re-invokes the child's factory to obtain a fresh mailbox, then
 * spawns a new wrapper coroutine that runs the actor loop and notifies
 * the supervisor on its exit.
 *
 * A simple intensity guard caps total restarts at JINN_SUP_MAX_RESTARTS
 * to avoid infinite restart storms.
 */
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
    void               *mb_ptr;       /* current mailbox (NULL = not running) */
    const char         *name;          /* informational, may be NULL */
    int                 alive;
} jinn_sup_child_slot_t;

struct jinn_sup {
    jinn_sup_strategy_t   strategy;
    jinn_sup_child_slot_t *children;
    size_t                 n_children;
    size_t                 cap_children;
    int                    restart_count;
    int                    started;
};

typedef struct {
    jinn_sup_t *sup;
    size_t      idx;
} jinn_sup_child_arg_t;

/* Forward */
static void jinn_sup_spawn_one(jinn_sup_t *sup, size_t idx);
static void sup_on_child_exit(void *arg);

jinn_sup_t *jinn_sup_create(jinn_sup_strategy_t strategy) {
    jinn_sup_t *s = (jinn_sup_t *)calloc(1, sizeof(*s));
    if (!s) return NULL;
    s->strategy = strategy;
    s->cap_children = 4;
    s->children = (jinn_sup_child_slot_t *)calloc(s->cap_children, sizeof(*s->children));
    if (!s->children) { free(s); return NULL; }
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
    /* Wrapper coroutine entry: run the actor loop, then on_exit_cb fires
     * and notifies the supervisor. We do NOT call jinn_actor_destroy here
     * because the supervisor reuses / replaces the mailbox via its factory. */
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
    /* on_exit_cb (sup_on_child_exit) will be called from jinn_coro_exit. */
}

static void jinn_sup_spawn_one(jinn_sup_t *sup, size_t idx) {
    if (!sup || idx >= sup->n_children) return;
    jinn_sup_child_slot_t *slot = &sup->children[idx];
    /* Reclaim previous mailbox if any (shouldn't normally happen — child
     * is dead before we restart). */
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
    jinn_sup_child_slot_t *slot = &sup->children[idx];
    slot->alive = 0;
    /* Free the dead mailbox so factory can produce a fresh one. */
    if (slot->mb_ptr) {
        jinn_actor_destroy(slot->mb_ptr);
        slot->mb_ptr = NULL;
    }
    if (sup->restart_count >= JINN_SUP_MAX_RESTARTS) {
        return;
    }
    sup->restart_count++;
    switch (sup->strategy) {
    case JINN_SUP_ONE_FOR_ONE:
        jinn_sup_spawn_one(sup, idx);
        break;
    case JINN_SUP_ONE_FOR_ALL:
        /* Stop every other live child; all will be restarted when their
         * loops notice the closed channel and exit. To avoid recursive
         * restart storms we mark restart_count up-front and stop them
         * synchronously here. Their on-exit will trigger sup_on_child_exit
         * but the strategy switch will only restart them individually if
         * still under the cap. Simpler: just restart this one and stop the
         * others; their natural exits will respawn them via OneForOne path. */
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
}

void jinn_sup_start(jinn_sup_t *sup) {
    if (!sup || sup->started) return;
    sup->started = 1;
    for (size_t i = 0; i < sup->n_children; i++) {
        jinn_sup_spawn_one(sup, i);
    }
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
    free(sup);
}
