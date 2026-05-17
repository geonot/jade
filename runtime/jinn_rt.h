/*
 * Jinn Runtime — Concurrency primitives for the Jinn language.
 *
 * Stackful coroutines, typed channels, M:N work-stealing scheduler,
 * actor support, select, timers.
 *
 * All functions prefixed with jinn_ to avoid symbol collisions.
 */
#pragma once

#include <stdint.h>
#include <stddef.h>
#include <stdatomic.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Forward declarations ────────────────────────────────────────── */

typedef struct jinn_coro    jinn_coro_t;
typedef struct jinn_sched   jinn_sched_t;
typedef struct jinn_chan     jinn_chan_t;
typedef struct jinn_worker  jinn_worker_t;
typedef struct jinn_deque   jinn_deque_t;
typedef struct jinn_timer   jinn_timer_t;

/* ── Persistence / store extension types (opaque) ────────────── */
typedef struct JinnBloom JinnBloom;
typedef struct JinnCol   JinnCol;
typedef struct JinnFts   JinnFts;
typedef struct JinnIndex JinnIndex;
typedef struct JinnKV    JinnKV;
typedef struct JinnVec   JinnVec;

/* ── Small-string optimization layout ─────────────────────────── */
typedef struct { char bytes[24]; } jinn_sso_t;

/* ── WAL replay callback ─────────────────────────────────────── */
typedef void (*jinn_wal_replay_cb)(uint8_t op, const void *payload,
                                   uint32_t payload_len, int64_t timestamp,
                                   void *user_data);

/* ── Context (platform-specific) ─────────────────────────────────── */

#if defined(__x86_64__) || defined(_M_X64)
typedef struct {
    void *rsp;
    void *rbp;
    void *rbx;
    void *r12;
    void *r13;
    void *r14;
    void *r15;
} jinn_context_t;  /* 56 bytes */
#elif defined(__aarch64__) || defined(_M_ARM64)
typedef struct {
    void *sp;
    void *lr;
    void *fp;
    void *x19_x28[10];
    double d8_d15[8];
} jinn_context_t;  /* 168 bytes */
#else
#include <setjmp.h>
typedef struct {
    jmp_buf env;
} jinn_context_t;
#endif

/* ── Coroutine ───────────────────────────────────────────────────── */

typedef enum {
    JINN_CORO_READY,
    JINN_CORO_RUNNING,
    JINN_CORO_SUSPENDED,
    JINN_CORO_DONE
} jinn_coro_state_t;

struct jinn_coro {
    jinn_context_t     ctx;
    void              *stack_base;
    uint32_t           stack_size;
    jinn_coro_state_t  state;
    void             (*entry)(void*);
    void              *arg;
    jinn_coro_t       *next;          /* intrusive list for wait queues */
    void              *wait_chan;      /* channel blocked on, or NULL */
    uint32_t           id;
    int                select_ready;  /* which select case fired (-1 = none) */
    uint8_t            daemon;        /* 1 = daemon coro (actor), doesn't block sched_run */
    void             (*on_exit_cb)(void *);  /* called when coro returns (before destroy) */
    void              *on_exit_arg;
};

#define JINN_STACK_SIZE  (64 * 1024)   /* 64KB per coroutine */
#define JINN_GUARD_SIZE  4096          /* 1 page guard */

jinn_coro_t *jinn_coro_create(void (*entry)(void*), void *arg);
void         jinn_coro_destroy(jinn_coro_t *c);
void         jinn_coro_yield(void);
void         jinn_coro_set_daemon(jinn_coro_t *c);
void         jinn_coro_set_on_exit(jinn_coro_t *c, void (*cb)(void *), void *arg);

/* ── Generator direct context-swap API ───────────────────────────── */

void jinn_gen_resume(void *gen_blk);
void jinn_gen_suspend(void *gen_blk);
void jinn_gen_destroy(void *gen_blk);

extern _Thread_local jinn_coro_t *tl_gen_coro;

/* ── Context switch (defined in assembly or fallback) ────────────── */

void jinn_context_swap(jinn_context_t *from, jinn_context_t *to);

/* ── Work-stealing deque ─────────────────────────────────────────── */

#define JINN_DEQUE_INIT_CAP 1024

struct jinn_deque {
    jinn_coro_t       **buffer;
    _Atomic(int64_t)    top;
    _Atomic(int64_t)    bottom;
    int64_t             capacity;
};

void         jinn_deque_init(jinn_deque_t *dq);
void         jinn_deque_destroy(jinn_deque_t *dq);
void         jinn_deque_push(jinn_deque_t *dq, jinn_coro_t *c);
jinn_coro_t *jinn_deque_pop(jinn_deque_t *dq);
jinn_coro_t *jinn_deque_steal(jinn_deque_t *dq);

/* ── Scheduler ───────────────────────────────────────────────────── */

/* Scheduler actions communicated from coroutine to scheduler across swap */
#define SCHED_ACTION_PARK    0  /* parked on wait queue — don't touch coroutine */
#define SCHED_ACTION_REQUEUE 1  /* voluntary yield — re-enqueue */
#define SCHED_ACTION_DESTROY 2  /* coroutine exited — destroy it */

struct jinn_worker {
    pthread_t          thread;
    uint32_t           id;
    jinn_deque_t       run_queue;
    jinn_coro_t       *current;
    jinn_context_t     sched_ctx;
    uint64_t           rng_state;
    void              *held_chan_lock;  /* channel lock held across context swap */
    int                last_action;     /* SCHED_ACTION_* set before swap */
};

struct jinn_sched {
    jinn_worker_t     *workers;
    int                num_workers;
    _Atomic(int64_t)   active_coros;
    _Atomic(int32_t)   shutdown;
    /* Global inject queue */
    jinn_coro_t       *inject_head;
    jinn_coro_t       *inject_tail;
    /* Idle parking */
    pthread_mutex_t    idle_lock;
    pthread_cond_t     idle_cond;
    _Atomic(int32_t)   idle_count;
    /* Started flag */
    _Atomic(int32_t)   started;
    /* Completion signaling — replaces usleep polling in jinn_sched_run */
    pthread_mutex_t    done_lock;
    pthread_cond_t     done_cond;
};

void jinn_sched_init(int num_workers);
void jinn_sched_spawn(jinn_coro_t *c);
void jinn_sched_run(void);
void jinn_sched_shutdown(void);
void jinn_sched_enqueue(jinn_coro_t *c);
void jinn_sched_yield(void);
void jinn_sched_park(void);
void jinn_sched_unpark(jinn_coro_t *c);

/* Get current coroutine (thread-local) */
jinn_coro_t  *jinn_current_coro(void);
jinn_worker_t *jinn_current_worker(void);

/* ── Channels ────────────────────────────────────────────────────── */

struct jinn_chan {
    _Atomic(uint64_t)  head;
    _Atomic(uint64_t)  tail;
    uint64_t           capacity;
    size_t             elem_size;
    void              *buffer;
    _Atomic(int32_t)   closed;
    jinn_coro_t       *send_waitq;
    jinn_coro_t       *send_waitq_tail;
    jinn_coro_t       *recv_waitq;
    jinn_coro_t       *recv_waitq_tail;
    _Atomic(int32_t)   lock;         /* spinlock (no thread-ownership tracking) */
};

jinn_chan_t *jinn_chan_create(size_t elem_size, size_t capacity);
void        jinn_chan_send(jinn_chan_t *ch, const void *data);
int         jinn_chan_recv(jinn_chan_t *ch, void *data_out);
int         jinn_chan_try_recv(jinn_chan_t *ch, void *data_out);
void        jinn_chan_close(jinn_chan_t *ch);
void        jinn_chan_destroy(jinn_chan_t *ch);

/* ── Select ──────────────────────────────────────────────────────── */

typedef struct jinn_select_case {
    jinn_chan_t   *chan;
    void          *data;
    int            is_send;
} jinn_select_case_t;

int jinn_select(jinn_select_case_t *cases, int n, int has_default);

/* ── Timers ──────────────────────────────────────────────────────── */

struct jinn_timer {
    uint64_t deadline_ns;
    int      fired;
};

void     jinn_timer_set(jinn_timer_t *t, uint64_t deadline_ns);
int      jinn_timer_check(jinn_timer_t *t);
uint64_t jinn_time_now_ns(void);

/* ── Pool allocator ──────────────────────────────────────────────── */

typedef struct jinn_pool jinn_pool_t;

jinn_pool_t *jinn_pool_create(size_t obj_size, size_t count);
void        *jinn_pool_alloc(jinn_pool_t *pool);
void         jinn_pool_free(jinn_pool_t *pool, void *ptr);
void         jinn_pool_reset(jinn_pool_t *pool);
void         jinn_pool_destroy(jinn_pool_t *pool);
size_t       jinn_pool_count(jinn_pool_t *pool);
size_t       jinn_pool_capacity(jinn_pool_t *pool);

/* ── Actor helpers ───────────────────────────────────────────────── */

void jinn_actor_park(void *mailbox_ptr);
void jinn_actor_wake(void *mailbox_ptr);
void jinn_actor_stop(void *mailbox_ptr);
void jinn_actor_destroy(void *mailbox_ptr);

/* ── Supervisor (OTP-style) ──────────────────────────────────────── */

typedef struct jinn_sup jinn_sup_t;

typedef enum {
    JINN_SUP_ONE_FOR_ONE = 0,
    JINN_SUP_ONE_FOR_ALL = 1,
    JINN_SUP_REST_FOR_ONE = 2,
} jinn_sup_strategy_t;

/* Factory: allocate + initialise a fresh mailbox for this child.
 * Returns the mailbox pointer. Must be called fresh for every (re)start. */
typedef void *(*jinn_sup_factory_t)(void);

/* Loop function (the actor's entry point). Takes mb_ptr. */
typedef void (*jinn_sup_loop_t)(void *);

jinn_sup_t *jinn_sup_create(jinn_sup_strategy_t strategy);
size_t      jinn_sup_register(jinn_sup_t *sup, jinn_sup_factory_t factory,
                              jinn_sup_loop_t loop_fn, const char *name);
void        jinn_sup_start(jinn_sup_t *sup);
int         jinn_sup_restart_count(jinn_sup_t *sup);
void        jinn_sup_destroy(jinn_sup_t *sup);
void       *jinn_sup_child_mailbox(jinn_sup_t *sup, size_t idx);

/* ── Global scheduler instance ───────────────────────────────────── */

extern jinn_sched_t g_sched;
extern _Thread_local jinn_worker_t *tl_worker;

/* ── Checked allocation ──────────────────────────────────────────── */

void *jinn_xmalloc(size_t size);
void jinn_store_truncation_warn(int64_t original_len, int64_t max_len);
void jinn_store_reserve(FILE *fp, int64_t count, int64_t rec_size);

/* ── Hashing ─────────────────────────────────────────────────────── */
uint64_t jinn_fnv1a(const void *data, int64_t len);

/* ── Process helpers ─────────────────────────────────────────────── */

long jinn_popen_read(const char *cmd, char *buf, long buf_size, int *exit_code);
int  jinn_system(const char *cmd);
long jinn_exec_capture(const char *prog, char *const argv[], char *buf, long buf_size, int *exit_code);
int  jinn_exec_argv(const char *prog, char *const argv[], int *exit_code);
int  jinn_exec_argv_timeout(const char *prog, char *const argv[], int *exit_code, long timeout_ms);
long jinn_exec_argv_capture(const char *prog, char *const argv[], char *buf, long buf_size, int *exit_code);
long jinn_exec_argv_capture_timeout(const char *prog, char *const argv[],
                                    char *buf, long buf_size, int *exit_code, long timeout_ms);

/* Vec<String>-aware spawn (called from std/process.jn) */
long jinn_spawn_capture(const void *vec_ptr, char *buf, long buf_size, int *exit_code);
int  jinn_spawn_exec(const void *vec_ptr, int *exit_code);


/* ── Auto-collected runtime FFI prototypes ──────────────── */
/* runtime/bloom.c */
JinnBloom *jinn_bloom_create(int64_t expected_items, double fp_rate);
JinnBloom *jinn_bloom_open(const char *path, int64_t expected_items);
void jinn_bloom_close(JinnBloom *b);
void jinn_bloom_add(JinnBloom *b, const void *data, int64_t len);
int64_t jinn_bloom_test(JinnBloom *b, const void *data, int64_t len);
void jinn_bloom_add_i64(JinnBloom *b, int64_t val);
int64_t jinn_bloom_test_i64(JinnBloom *b, int64_t val);
void jinn_bloom_add_str(JinnBloom *b, const char *data, int64_t len);
int64_t jinn_bloom_test_str(JinnBloom *b, const char *data, int64_t len);
/* runtime/column.c */
JinnCol *jinn_col_open(const char *path, int64_t elem_size);
void jinn_col_close(JinnCol *c);
void jinn_col_append(JinnCol *c, const void *data);
int64_t jinn_col_count(JinnCol *c);
int64_t jinn_col_read_all(JinnCol *c, void *buf, int64_t max_elems);
int64_t jinn_col_sum_i64(JinnCol *c);
int64_t jinn_col_min_i64(JinnCol *c);
int64_t jinn_col_max_i64(JinnCol *c);
int64_t jinn_col_avg_sum_i64(JinnCol *c);
double jinn_col_sum_f64(JinnCol *c);
double jinn_col_min_f64(JinnCol *c);
double jinn_col_max_f64(JinnCol *c);
int64_t jinn_col_distinct_i64(JinnCol *c);
/* runtime/event.c */
void *jinn_event_loop_create(int max_events);
void jinn_event_loop_destroy(void *handle);
int jinn_fd_set_nonblock(int fd);
int jinn_event_loop_add_read(void *handle, int fd, void *waiter_ptr);
int jinn_event_loop_add_write(void *handle, int fd, void *waiter_ptr);
int jinn_event_loop_rearm_read(void *handle, int fd, void *waiter_ptr);
int jinn_event_loop_rearm_write(void *handle, int fd, void *waiter_ptr);
int jinn_event_loop_remove(void *handle, int fd);
int jinn_event_loop_poll(void *handle, int timeout_ms, int *ready_fds, int *ready_events, int max_ready);
int jinn_event_wait_readable(int fd, int timeout_ms);
int jinn_event_wait_writable(int fd, int timeout_ms);
void *jinn_io_waiter_create(int fd);
void jinn_io_waiter_destroy(void *waiter);
void jinn_io_waiter_set_coro(void *waiter, void *coro);
/* runtime/fs.c */
int c_mkdir(const char *path, int mode);
int c_rmdir(const char *path);
int c_remove(const char *path);
int c_rename(const char *old, const char *new_name);
int c_chdir(const char *path);
int c_symlink(const char *target, const char *linkpath);
const char *jinn_dirent_name(void *ent);
int jinn_is_dir(const char *path);
int jinn_is_file(const char *path);
int jinn_is_symlink(const char *path);
long jinn_file_mtime(const char *path);
long jinn_file_size(const char *path);
long fstat_size(int fd);
int jinn_fd_close(int fd);
int jinn_chmod(const char *path, int mode);
double c_hypot(double x, double y);
const char *jinn_hostname(void);
const char *jinn_cwd(void);
/* runtime/fts.c */
JinnFts *jinn_fts_open(const char *path);
void jinn_fts_close(JinnFts *f);
void jinn_fts_add(JinnFts *f, int64_t doc_id, const char *text, int64_t text_len);
int64_t jinn_fts_search(JinnFts *f, const char *query, int64_t *out_ids, int64_t max_ids);
int64_t jinn_fts_count(JinnFts *f, const char *query);
int64_t jinn_fts_search_n(JinnFts *f, const char *query, int64_t qlen);
int64_t jinn_fts_count_n(JinnFts *f, const char *query, int64_t qlen);
void jinn_fts_add_n(JinnFts *f, int64_t doc_id, const char *text, int64_t text_len);
int64_t jinn_fts_posting_count(JinnFts *f);
/* runtime/index.c */
uint64_t jinn_idx_hash_i64(int64_t val);
uint64_t jinn_idx_hash_str(const char *buf, int64_t len);
uint64_t jinn_idx_hash_f64(double val);
JinnIndex *jinn_idx_open(const char *path);
void jinn_idx_close(JinnIndex *idx);
void jinn_idx_insert(JinnIndex *idx, uint64_t hash, int64_t record_offset);
int64_t jinn_idx_lookup(JinnIndex *idx, uint64_t hash);
int jinn_idx_contains(JinnIndex *idx, uint64_t hash);
void jinn_idx_delete(JinnIndex *idx, uint64_t hash);
void jinn_idx_clear(JinnIndex *idx);
/* runtime/kv.c */
JinnKV *jinn_kv_open(const char *path);
void jinn_kv_close(JinnKV *kv);
void jinn_kv_set(JinnKV *kv, const char *key, int64_t key_len, int64_t value);
int64_t jinn_kv_get(JinnKV *kv, const char *key, int64_t key_len);
int jinn_kv_has(JinnKV *kv, const char *key, int64_t key_len);
void jinn_kv_del(JinnKV *kv, const char *key, int64_t key_len);
void jinn_kv_incr(JinnKV *kv, const char *key, int64_t key_len, int64_t delta);
int64_t jinn_kv_count(JinnKV *kv);
void jinn_kv_persist(JinnKV *kv);
/* runtime/migrate.c */
FILE *jinn_mig_log_open(const char *path);
void jinn_mig_log_close(FILE *fp);
int64_t jinn_mig_log_applied(FILE *fp, int64_t version);
void jinn_mig_log_record(FILE *fp, int64_t version, int64_t direction);
int64_t jinn_mig_add_field(FILE **store_fp_ptr, const char *store_path, int64_t field_offset, int64_t field_size, const void *default_val);
int64_t jinn_mig_drop_field(FILE **store_fp_ptr, const char *store_path, int64_t field_offset, int64_t field_size);
/* runtime/net.c */
int jinn_socket(int domain, int type, int protocol);
int jinn_close(int fd);
int listen_sock(int fd, int backlog);
long jinn_send(int fd, const void *buf, long len, int flags);
long jinn_recv(int fd, void *buf, long len, int flags);
long jinn_sendto(int fd, const void *buf, long len, int flags, const void *addr, int addrlen);
long jinn_recvfrom(int fd, void *buf, long len, int flags, void *addr, int *addrlen);
/* runtime/pool.c */
/* runtime/process.c */
/* runtime/regex_helper.c */
int64_t jinn_ovector_get(void *ovector, int64_t idx);
/* runtime/util.c */
int64_t jinn_f64_to_bits(double val);
double jinn_bits_to_f64(int64_t bits);
const char *jinn_getenv_or_empty(const char *name);
void jinn_sort_i64(int64_t *data, int64_t len);
void jinn_sort_f64(double *data, int64_t len);
/* runtime/terminal.c */
int jinn_terminal_enable_raw(int fd);
int jinn_terminal_disable_raw(int fd);
int jinn_terminal_size(int32_t *out_cols, int32_t *out_rows);
/* runtime/vec.c */
void *__jinn_vec_slice(void *hdr, int64_t start, int64_t end, int64_t elem_size);
void *__jinn_vec_clone_pod(void *hdr, int64_t elem_size);
jinn_sso_t __jinn_str_slice(jinn_sso_t str, int64_t start, int64_t end);
void __jinn_str_clone(jinn_sso_t *out, const jinn_sso_t *src);
void *__jinn_deque_new(void);
void __jinn_deque_push_back(void *handle, int64_t val);
void __jinn_deque_push_front(void *handle, int64_t val);
int64_t __jinn_deque_pop_front(void *handle);
int64_t __jinn_deque_pop_back(void *handle);
int64_t __jinn_deque_len(void *handle);
/* runtime/vector.c */
JinnVec *jinn_vec_open(const char *path, int64_t dims);
void jinn_vec_close(JinnVec *v);
void jinn_vec_insert(JinnVec *v, const double *vec);
int64_t jinn_vec_count(JinnVec *v);
int64_t jinn_vec_nearest(JinnVec *v, const double *query, int64_t k, int64_t *out_indices);
/* runtime/version.c */
FILE *jinn_ver_open(const char *path);
void jinn_ver_close(FILE *f);
void jinn_ver_append(FILE *f, int64_t sid, int64_t version, const void *record_data, int64_t rec_size);
int64_t jinn_ver_count(FILE *f, int64_t sid, int64_t rec_size);
int64_t jinn_ver_at(FILE *f, int64_t sid, int64_t version, void *out_buf, int64_t rec_size);
int64_t jinn_ver_history(FILE *f, int64_t sid, void *out_buf, int64_t rec_size, int64_t max_versions);
void jinn_ver_compact(FILE *f, int64_t rec_size, int64_t keep_n);
/* runtime/wal.c */
void jinn_wal_commit_group(FILE *wal);
FILE *jinn_wal_open(const char *path);
void jinn_wal_write(FILE *wal, uint8_t op, const void *payload, uint32_t payload_len);
void jinn_wal_checkpoint(FILE *wal);
void jinn_wal_close(FILE *wal);
int64_t jinn_wal_size(FILE *wal);
int64_t jinn_wal_replay(FILE *wal, jinn_wal_replay_cb callback, void *user_data);

/* ── Optional modules (only linked when feature available) ── */
/* runtime/crypto.c (requires OpenSSL) */
int  jinn_sha256(const unsigned char *data, long len, unsigned char *out);
int  jinn_sha512(const unsigned char *data, long len, unsigned char *out);
int  jinn_hmac_sha256(const unsigned char *key, long key_len,
                      const unsigned char *data, long data_len,
                      unsigned char *out);
long jinn_aes_gcm_encrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *plaintext, long pt_len,
                          const unsigned char *aad, long aad_len,
                          unsigned char *out, unsigned char *tag_out);
long jinn_aes_gcm_decrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *ciphertext, long ct_len,
                          const unsigned char *aad, long aad_len,
                          const unsigned char *tag, unsigned char *out);
int  jinn_random_bytes(unsigned char *buf, long n);
void jinn_bytes_to_hex(const unsigned char *data, long len, char *out);
long jinn_hex_to_bytes(const char *hex, long hex_len, unsigned char *out);
long jinn_evp_digest(const char *alg, const unsigned char *data, long len,
                     unsigned char *out);
long jinn_evp_hmac(const char *alg, const unsigned char *key, long key_len,
                   const unsigned char *data, long data_len,
                   unsigned char *out);
int  jinn_pbkdf2(const char *alg, const unsigned char *pass, long pass_len,
                 const unsigned char *salt, long salt_len, long iters,
                 long dklen, unsigned char *out);
long jinn_aes_cbc_encrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *pt, long pt_len,
                          unsigned char *out);
long jinn_aes_cbc_decrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *ct, long ct_len,
                          unsigned char *out);
long jinn_chacha20_poly1305_encrypt(const unsigned char *key,
                                    const unsigned char *nonce,
                                    const unsigned char *pt, long pt_len,
                                    const unsigned char *aad, long aad_len,
                                    unsigned char *out, unsigned char *tag);
long jinn_chacha20_poly1305_decrypt(const unsigned char *key,
                                    const unsigned char *nonce,
                                    const unsigned char *ct, long ct_len,
                                    const unsigned char *aad, long aad_len,
                                    const unsigned char *tag,
                                    unsigned char *out);
int  jinn_argon2id(const unsigned char *pass, long pass_len,
                   const unsigned char *salt, long salt_len,
                   long t_cost, long m_cost_kib, long parallelism,
                   long dklen, unsigned char *out);
int  jinn_scrypt(const unsigned char *pass, long pass_len,
                 const unsigned char *salt, long salt_len,
                 long n, long r, long p, long dklen, unsigned char *out);

/* runtime/tls.c (requires OpenSSL) */
typedef struct jinn_tls_conn jinn_tls_conn;
void           jinn_tls_init(void);
jinn_tls_conn *jinn_tls_connect(const char *host, int port);
long           jinn_tls_send(jinn_tls_conn *conn, const char *buf, long len);
long           jinn_tls_recv(jinn_tls_conn *conn, char *buf, long len);
void           jinn_tls_close(jinn_tls_conn *conn);
typedef struct jinn_tls_listener jinn_tls_listener;
jinn_tls_listener *jinn_tls_listen(const char *host, int port,
                                   const char *cert_path,
                                   const char *key_path);
jinn_tls_conn *jinn_tls_accept(jinn_tls_listener *l);
void           jinn_tls_listener_close(jinn_tls_listener *l);
long           jinn_tls_last_error(char *buf, long len);
long           jinn_tls_peer_cert_subject(jinn_tls_conn *conn, char *buf, long len);
long           jinn_tls_protocol_version(jinn_tls_conn *conn, char *buf, long len);
int            jinn_dns_resolve(const char *host, char *out_buf, int out_len);
int            jinn_dns_resolve_all(const char *host, char *out_buf, int out_len);

/* runtime/sqlite.c (requires sqlite3) */
void       *jinn_sqlite_open(const char *path);
int         jinn_sqlite_close(void *db);
int         jinn_sqlite_exec(void *db, const char *sql);
const char *jinn_sqlite_errmsg(void *db);
long        jinn_sqlite_last_insert_id(void *db);
long        jinn_sqlite_changes(void *db);
void       *jinn_sqlite_prepare(void *db, const char *sql);
void        jinn_sqlite_finalize(void *stmt);
int         jinn_sqlite_reset(void *stmt);
int         jinn_sqlite_bind_int(void *stmt, int idx, long val);
int         jinn_sqlite_bind_float(void *stmt, int idx, double val);
int         jinn_sqlite_bind_text(void *stmt, int idx, const char *val, long len);
int         jinn_sqlite_bind_null(void *stmt, int idx);
int         jinn_sqlite_bind_blob(void *stmt, int idx, const void *data, long len);
int         jinn_sqlite_step(void *stmt);
int         jinn_sqlite_column_count(void *stmt);
const char *jinn_sqlite_column_name(void *stmt, int idx);
int         jinn_sqlite_column_type(void *stmt, int idx);
long        jinn_sqlite_column_int(void *stmt, int idx);
double      jinn_sqlite_column_float(void *stmt, int idx);
const char *jinn_sqlite_column_text(void *stmt, int idx);
long        jinn_sqlite_column_text_len(void *stmt, int idx);
const void *jinn_sqlite_column_blob(void *stmt, int idx);
long        jinn_sqlite_column_blob_len(void *stmt, int idx);
int         jinn_sqlite_begin(void *db);
int         jinn_sqlite_commit(void *db);
int         jinn_sqlite_rollback(void *db);

#ifdef __cplusplus
}
#endif
