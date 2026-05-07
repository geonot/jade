/*
 * Jade Runtime — Concurrency primitives for the Jade language.
 *
 * Stackful coroutines, typed channels, M:N work-stealing scheduler,
 * actor support, select, timers.
 *
 * All functions prefixed with jade_ to avoid symbol collisions.
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

typedef struct jade_coro    jade_coro_t;
typedef struct jade_sched   jade_sched_t;
typedef struct jade_chan     jade_chan_t;
typedef struct jade_worker  jade_worker_t;
typedef struct jade_deque   jade_deque_t;
typedef struct jade_timer   jade_timer_t;

/* ── Persistence / store extension types (opaque) ────────────── */
typedef struct JadeBloom JadeBloom;
typedef struct JadeCol   JadeCol;
typedef struct JadeFts   JadeFts;
typedef struct JadeIndex JadeIndex;
typedef struct JadeKV    JadeKV;
typedef struct JadeVec   JadeVec;

/* ── Small-string optimization layout ─────────────────────────── */
typedef struct { char bytes[24]; } jade_sso_t;

/* ── WAL replay callback ─────────────────────────────────────── */
typedef void (*jade_wal_replay_cb)(uint8_t op, const void *payload,
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
} jade_context_t;  /* 56 bytes */
#elif defined(__aarch64__) || defined(_M_ARM64)
typedef struct {
    void *sp;
    void *lr;
    void *fp;
    void *x19_x28[10];
    double d8_d15[8];
} jade_context_t;  /* 168 bytes */
#else
#include <setjmp.h>
typedef struct {
    jmp_buf env;
} jade_context_t;
#endif

/* ── Coroutine ───────────────────────────────────────────────────── */

typedef enum {
    JADE_CORO_READY,
    JADE_CORO_RUNNING,
    JADE_CORO_SUSPENDED,
    JADE_CORO_DONE
} jade_coro_state_t;

struct jade_coro {
    jade_context_t     ctx;
    void              *stack_base;
    uint32_t           stack_size;
    jade_coro_state_t  state;
    void             (*entry)(void*);
    void              *arg;
    jade_coro_t       *next;          /* intrusive list for wait queues */
    void              *wait_chan;      /* channel blocked on, or NULL */
    uint32_t           id;
    int                select_ready;  /* which select case fired (-1 = none) */
    uint8_t            daemon;        /* 1 = daemon coro (actor), doesn't block sched_run */
    void             (*on_exit_cb)(void *);  /* called when coro returns (before destroy) */
    void              *on_exit_arg;
};

#define JADE_STACK_SIZE  (64 * 1024)   /* 64KB per coroutine */
#define JADE_GUARD_SIZE  4096          /* 1 page guard */

jade_coro_t *jade_coro_create(void (*entry)(void*), void *arg);
void         jade_coro_destroy(jade_coro_t *c);
void         jade_coro_yield(void);
void         jade_coro_set_daemon(jade_coro_t *c);
void         jade_coro_set_on_exit(jade_coro_t *c, void (*cb)(void *), void *arg);

/* ── Generator direct context-swap API ───────────────────────────── */

void jade_gen_resume(void *gen_blk);
void jade_gen_suspend(void *gen_blk);
void jade_gen_destroy(void *gen_blk);

extern _Thread_local jade_coro_t *tl_gen_coro;

/* ── Context switch (defined in assembly or fallback) ────────────── */

void jade_context_swap(jade_context_t *from, jade_context_t *to);

/* ── Work-stealing deque ─────────────────────────────────────────── */

#define JADE_DEQUE_INIT_CAP 1024

struct jade_deque {
    jade_coro_t       **buffer;
    _Atomic(int64_t)    top;
    _Atomic(int64_t)    bottom;
    int64_t             capacity;
};

void         jade_deque_init(jade_deque_t *dq);
void         jade_deque_destroy(jade_deque_t *dq);
void         jade_deque_push(jade_deque_t *dq, jade_coro_t *c);
jade_coro_t *jade_deque_pop(jade_deque_t *dq);
jade_coro_t *jade_deque_steal(jade_deque_t *dq);

/* ── Scheduler ───────────────────────────────────────────────────── */

/* Scheduler actions communicated from coroutine to scheduler across swap */
#define SCHED_ACTION_PARK    0  /* parked on wait queue — don't touch coroutine */
#define SCHED_ACTION_REQUEUE 1  /* voluntary yield — re-enqueue */
#define SCHED_ACTION_DESTROY 2  /* coroutine exited — destroy it */

struct jade_worker {
    pthread_t          thread;
    uint32_t           id;
    jade_deque_t       run_queue;
    jade_coro_t       *current;
    jade_context_t     sched_ctx;
    uint64_t           rng_state;
    void              *held_chan_lock;  /* channel lock held across context swap */
    int                last_action;     /* SCHED_ACTION_* set before swap */
};

struct jade_sched {
    jade_worker_t     *workers;
    int                num_workers;
    _Atomic(int64_t)   active_coros;
    _Atomic(int32_t)   shutdown;
    /* Global inject queue */
    jade_coro_t       *inject_head;
    jade_coro_t       *inject_tail;
    /* Idle parking */
    pthread_mutex_t    idle_lock;
    pthread_cond_t     idle_cond;
    _Atomic(int32_t)   idle_count;
    /* Started flag */
    _Atomic(int32_t)   started;
    /* Completion signaling — replaces usleep polling in jade_sched_run */
    pthread_mutex_t    done_lock;
    pthread_cond_t     done_cond;
};

void jade_sched_init(int num_workers);
void jade_sched_spawn(jade_coro_t *c);
void jade_sched_run(void);
void jade_sched_shutdown(void);
void jade_sched_enqueue(jade_coro_t *c);
void jade_sched_yield(void);
void jade_sched_park(void);
void jade_sched_unpark(jade_coro_t *c);

/* Get current coroutine (thread-local) */
jade_coro_t  *jade_current_coro(void);
jade_worker_t *jade_current_worker(void);

/* ── Channels ────────────────────────────────────────────────────── */

struct jade_chan {
    _Atomic(uint64_t)  head;
    _Atomic(uint64_t)  tail;
    uint64_t           capacity;
    size_t             elem_size;
    void              *buffer;
    _Atomic(int32_t)   closed;
    jade_coro_t       *send_waitq;
    jade_coro_t       *send_waitq_tail;
    jade_coro_t       *recv_waitq;
    jade_coro_t       *recv_waitq_tail;
    _Atomic(int32_t)   lock;         /* spinlock (no thread-ownership tracking) */
};

jade_chan_t *jade_chan_create(size_t elem_size, size_t capacity);
void        jade_chan_send(jade_chan_t *ch, const void *data);
int         jade_chan_recv(jade_chan_t *ch, void *data_out);
int         jade_chan_try_recv(jade_chan_t *ch, void *data_out);
void        jade_chan_close(jade_chan_t *ch);
void        jade_chan_destroy(jade_chan_t *ch);

/* ── Select ──────────────────────────────────────────────────────── */

typedef struct jade_select_case {
    jade_chan_t   *chan;
    void          *data;
    int            is_send;
} jade_select_case_t;

int jade_select(jade_select_case_t *cases, int n, int has_default);

/* ── Timers ──────────────────────────────────────────────────────── */

struct jade_timer {
    uint64_t deadline_ns;
    int      fired;
};

void     jade_timer_set(jade_timer_t *t, uint64_t deadline_ns);
int      jade_timer_check(jade_timer_t *t);
uint64_t jade_time_now_ns(void);

/* ── Pool allocator ──────────────────────────────────────────────── */

typedef struct jade_pool jade_pool_t;

jade_pool_t *jade_pool_create(size_t obj_size, size_t count);
void        *jade_pool_alloc(jade_pool_t *pool);
void         jade_pool_free(jade_pool_t *pool, void *ptr);
void         jade_pool_reset(jade_pool_t *pool);
void         jade_pool_destroy(jade_pool_t *pool);
size_t       jade_pool_count(jade_pool_t *pool);
size_t       jade_pool_capacity(jade_pool_t *pool);

/* ── Actor helpers ───────────────────────────────────────────────── */

void jade_actor_park(void *mailbox_ptr);
void jade_actor_wake(void *mailbox_ptr);
void jade_actor_stop(void *mailbox_ptr);
void jade_actor_destroy(void *mailbox_ptr);

/* ── Supervisor (OTP-style) ──────────────────────────────────────── */

typedef struct jade_sup jade_sup_t;

typedef enum {
    JADE_SUP_ONE_FOR_ONE = 0,
    JADE_SUP_ONE_FOR_ALL = 1,
    JADE_SUP_REST_FOR_ONE = 2,
} jade_sup_strategy_t;

/* Factory: allocate + initialise a fresh mailbox for this child.
 * Returns the mailbox pointer. Must be called fresh for every (re)start. */
typedef void *(*jade_sup_factory_t)(void);

/* Loop function (the actor's entry point). Takes mb_ptr. */
typedef void (*jade_sup_loop_t)(void *);

jade_sup_t *jade_sup_create(jade_sup_strategy_t strategy);
size_t      jade_sup_register(jade_sup_t *sup, jade_sup_factory_t factory,
                              jade_sup_loop_t loop_fn, const char *name);
void        jade_sup_start(jade_sup_t *sup);
int         jade_sup_restart_count(jade_sup_t *sup);
void        jade_sup_destroy(jade_sup_t *sup);
void       *jade_sup_child_mailbox(jade_sup_t *sup, size_t idx);

/* ── Global scheduler instance ───────────────────────────────────── */

extern jade_sched_t g_sched;
extern _Thread_local jade_worker_t *tl_worker;

/* ── Checked allocation ──────────────────────────────────────────── */

void *jade_xmalloc(size_t size);
void jade_store_truncation_warn(int64_t original_len, int64_t max_len);
void jade_store_reserve(FILE *fp, int64_t count, int64_t rec_size);

/* ── Hashing ─────────────────────────────────────────────────────── */
uint64_t jade_fnv1a(const void *data, int64_t len);

/* ── Process helpers ─────────────────────────────────────────────── */

long jade_popen_read(const char *cmd, char *buf, long buf_size, int *exit_code);
int  jade_system(const char *cmd);
long jade_exec_capture(const char *prog, char *const argv[], char *buf, long buf_size, int *exit_code);
int  jade_exec_argv(const char *prog, char *const argv[], int *exit_code);
int  jade_exec_argv_timeout(const char *prog, char *const argv[], int *exit_code, long timeout_ms);
long jade_exec_argv_capture(const char *prog, char *const argv[], char *buf, long buf_size, int *exit_code);
long jade_exec_argv_capture_timeout(const char *prog, char *const argv[],
                                    char *buf, long buf_size, int *exit_code, long timeout_ms);

/* Vec<String>-aware spawn (called from std/process.jade) */
long jade_spawn_capture(const void *vec_ptr, char *buf, long buf_size, int *exit_code);
int  jade_spawn_exec(const void *vec_ptr, int *exit_code);


/* ── Auto-collected runtime FFI prototypes ──────────────── */
/* runtime/bloom.c */
JadeBloom *jade_bloom_create(int64_t expected_items, double fp_rate);
JadeBloom *jade_bloom_open(const char *path, int64_t expected_items);
void jade_bloom_close(JadeBloom *b);
void jade_bloom_add(JadeBloom *b, const void *data, int64_t len);
int64_t jade_bloom_test(JadeBloom *b, const void *data, int64_t len);
void jade_bloom_add_i64(JadeBloom *b, int64_t val);
int64_t jade_bloom_test_i64(JadeBloom *b, int64_t val);
void jade_bloom_add_str(JadeBloom *b, const char *data, int64_t len);
int64_t jade_bloom_test_str(JadeBloom *b, const char *data, int64_t len);
/* runtime/column.c */
JadeCol *jade_col_open(const char *path, int64_t elem_size);
void jade_col_close(JadeCol *c);
void jade_col_append(JadeCol *c, const void *data);
int64_t jade_col_count(JadeCol *c);
int64_t jade_col_read_all(JadeCol *c, void *buf, int64_t max_elems);
int64_t jade_col_sum_i64(JadeCol *c);
int64_t jade_col_min_i64(JadeCol *c);
int64_t jade_col_max_i64(JadeCol *c);
int64_t jade_col_avg_sum_i64(JadeCol *c);
double jade_col_sum_f64(JadeCol *c);
double jade_col_min_f64(JadeCol *c);
double jade_col_max_f64(JadeCol *c);
int64_t jade_col_distinct_i64(JadeCol *c);
/* runtime/event.c */
void *jade_event_loop_create(int max_events);
void jade_event_loop_destroy(void *handle);
int jade_fd_set_nonblock(int fd);
int jade_event_loop_add_read(void *handle, int fd, void *waiter_ptr);
int jade_event_loop_add_write(void *handle, int fd, void *waiter_ptr);
int jade_event_loop_rearm_read(void *handle, int fd, void *waiter_ptr);
int jade_event_loop_rearm_write(void *handle, int fd, void *waiter_ptr);
int jade_event_loop_remove(void *handle, int fd);
int jade_event_loop_poll(void *handle, int timeout_ms, int *ready_fds, int *ready_events, int max_ready);
int jade_event_wait_readable(int fd, int timeout_ms);
int jade_event_wait_writable(int fd, int timeout_ms);
void *jade_io_waiter_create(int fd);
void jade_io_waiter_destroy(void *waiter);
void jade_io_waiter_set_coro(void *waiter, void *coro);
/* runtime/fs.c */
int c_mkdir(const char *path, int mode);
int c_rmdir(const char *path);
int c_remove(const char *path);
int c_rename(const char *old, const char *new_name);
int c_chdir(const char *path);
int c_symlink(const char *target, const char *linkpath);
const char *jade_dirent_name(void *ent);
int jade_is_dir(const char *path);
int jade_is_file(const char *path);
int jade_is_symlink(const char *path);
long jade_file_mtime(const char *path);
long jade_file_size(const char *path);
long fstat_size(int fd);
int jade_fd_close(int fd);
int jade_chmod(const char *path, int mode);
double c_hypot(double x, double y);
const char *jade_hostname(void);
const char *jade_cwd(void);
/* runtime/fts.c */
JadeFts *jade_fts_open(const char *path);
void jade_fts_close(JadeFts *f);
void jade_fts_add(JadeFts *f, int64_t doc_id, const char *text, int64_t text_len);
int64_t jade_fts_search(JadeFts *f, const char *query, int64_t *out_ids, int64_t max_ids);
int64_t jade_fts_count(JadeFts *f, const char *query);
int64_t jade_fts_search_n(JadeFts *f, const char *query, int64_t qlen);
int64_t jade_fts_count_n(JadeFts *f, const char *query, int64_t qlen);
void jade_fts_add_n(JadeFts *f, int64_t doc_id, const char *text, int64_t text_len);
int64_t jade_fts_posting_count(JadeFts *f);
/* runtime/index.c */
uint64_t jade_idx_hash_i64(int64_t val);
uint64_t jade_idx_hash_str(const char *buf, int64_t len);
uint64_t jade_idx_hash_f64(double val);
JadeIndex *jade_idx_open(const char *path);
void jade_idx_close(JadeIndex *idx);
void jade_idx_insert(JadeIndex *idx, uint64_t hash, int64_t record_offset);
int64_t jade_idx_lookup(JadeIndex *idx, uint64_t hash);
int jade_idx_contains(JadeIndex *idx, uint64_t hash);
void jade_idx_delete(JadeIndex *idx, uint64_t hash);
void jade_idx_clear(JadeIndex *idx);
/* runtime/kv.c */
JadeKV *jade_kv_open(const char *path);
void jade_kv_close(JadeKV *kv);
void jade_kv_set(JadeKV *kv, const char *key, int64_t key_len, int64_t value);
int64_t jade_kv_get(JadeKV *kv, const char *key, int64_t key_len);
int jade_kv_has(JadeKV *kv, const char *key, int64_t key_len);
void jade_kv_del(JadeKV *kv, const char *key, int64_t key_len);
void jade_kv_incr(JadeKV *kv, const char *key, int64_t key_len, int64_t delta);
int64_t jade_kv_count(JadeKV *kv);
void jade_kv_persist(JadeKV *kv);
/* runtime/migrate.c */
FILE *jade_mig_log_open(const char *path);
void jade_mig_log_close(FILE *fp);
int64_t jade_mig_log_applied(FILE *fp, int64_t version);
void jade_mig_log_record(FILE *fp, int64_t version, int64_t direction);
int64_t jade_mig_add_field(FILE **store_fp_ptr, const char *store_path, int64_t field_offset, int64_t field_size, const void *default_val);
int64_t jade_mig_drop_field(FILE **store_fp_ptr, const char *store_path, int64_t field_offset, int64_t field_size);
/* runtime/net.c */
int jade_socket(int domain, int type, int protocol);
int jade_close(int fd);
int listen_sock(int fd, int backlog);
long jade_send(int fd, const void *buf, long len, int flags);
long jade_recv(int fd, void *buf, long len, int flags);
long jade_sendto(int fd, const void *buf, long len, int flags, const void *addr, int addrlen);
long jade_recvfrom(int fd, void *buf, long len, int flags, void *addr, int *addrlen);
/* runtime/pool.c */
/* runtime/process.c */
/* runtime/regex_helper.c */
int64_t jade_ovector_get(void *ovector, int64_t idx);
/* runtime/util.c */
int64_t jade_f64_to_bits(double val);
double jade_bits_to_f64(int64_t bits);
const char *jade_getenv_or_empty(const char *name);
void jade_sort_i64(int64_t *data, int64_t len);
void jade_sort_f64(double *data, int64_t len);
/* runtime/terminal.c */
int jade_terminal_enable_raw(int fd);
int jade_terminal_disable_raw(int fd);
int jade_terminal_size(int32_t *out_cols, int32_t *out_rows);
/* runtime/vec.c */
void *__jade_vec_slice(void *hdr, int64_t start, int64_t end, int64_t elem_size);
jade_sso_t __jade_str_slice(jade_sso_t str, int64_t start, int64_t end);
void *__jade_deque_new(void);
void __jade_deque_push_back(void *handle, int64_t val);
void __jade_deque_push_front(void *handle, int64_t val);
int64_t __jade_deque_pop_front(void *handle);
int64_t __jade_deque_pop_back(void *handle);
int64_t __jade_deque_len(void *handle);
/* runtime/vector.c */
JadeVec *jade_vec_open(const char *path, int64_t dims);
void jade_vec_close(JadeVec *v);
void jade_vec_insert(JadeVec *v, const double *vec);
int64_t jade_vec_count(JadeVec *v);
int64_t jade_vec_nearest(JadeVec *v, const double *query, int64_t k, int64_t *out_indices);
/* runtime/version.c */
FILE *jade_ver_open(const char *path);
void jade_ver_close(FILE *f);
void jade_ver_append(FILE *f, int64_t sid, int64_t version, const void *record_data, int64_t rec_size);
int64_t jade_ver_count(FILE *f, int64_t sid, int64_t rec_size);
int64_t jade_ver_at(FILE *f, int64_t sid, int64_t version, void *out_buf, int64_t rec_size);
int64_t jade_ver_history(FILE *f, int64_t sid, void *out_buf, int64_t rec_size, int64_t max_versions);
void jade_ver_compact(FILE *f, int64_t rec_size, int64_t keep_n);
/* runtime/wal.c */
void jade_wal_commit_group(FILE *wal);
FILE *jade_wal_open(const char *path);
void jade_wal_write(FILE *wal, uint8_t op, const void *payload, uint32_t payload_len);
void jade_wal_checkpoint(FILE *wal);
void jade_wal_close(FILE *wal);
int64_t jade_wal_size(FILE *wal);
int64_t jade_wal_replay(FILE *wal, jade_wal_replay_cb callback, void *user_data);

/* ── Optional modules (only linked when feature available) ── */
/* runtime/crypto.c (requires OpenSSL) */
int  jade_sha256(const unsigned char *data, long len, unsigned char *out);
int  jade_sha512(const unsigned char *data, long len, unsigned char *out);
int  jade_hmac_sha256(const unsigned char *key, long key_len,
                      const unsigned char *data, long data_len,
                      unsigned char *out);
long jade_aes_gcm_encrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *plaintext, long pt_len,
                          const unsigned char *aad, long aad_len,
                          unsigned char *out, unsigned char *tag_out);
long jade_aes_gcm_decrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *ciphertext, long ct_len,
                          const unsigned char *aad, long aad_len,
                          const unsigned char *tag, unsigned char *out);
int  jade_random_bytes(unsigned char *buf, long n);
void jade_bytes_to_hex(const unsigned char *data, long len, char *out);
long jade_hex_to_bytes(const char *hex, long hex_len, unsigned char *out);
long jade_evp_digest(const char *alg, const unsigned char *data, long len,
                     unsigned char *out);
long jade_evp_hmac(const char *alg, const unsigned char *key, long key_len,
                   const unsigned char *data, long data_len,
                   unsigned char *out);
int  jade_pbkdf2(const char *alg, const unsigned char *pass, long pass_len,
                 const unsigned char *salt, long salt_len, long iters,
                 long dklen, unsigned char *out);
long jade_aes_cbc_encrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *pt, long pt_len,
                          unsigned char *out);
long jade_aes_cbc_decrypt(const unsigned char *key, const unsigned char *iv,
                          const unsigned char *ct, long ct_len,
                          unsigned char *out);
long jade_chacha20_poly1305_encrypt(const unsigned char *key,
                                    const unsigned char *nonce,
                                    const unsigned char *pt, long pt_len,
                                    const unsigned char *aad, long aad_len,
                                    unsigned char *out, unsigned char *tag);
long jade_chacha20_poly1305_decrypt(const unsigned char *key,
                                    const unsigned char *nonce,
                                    const unsigned char *ct, long ct_len,
                                    const unsigned char *aad, long aad_len,
                                    const unsigned char *tag,
                                    unsigned char *out);
int  jade_argon2id(const unsigned char *pass, long pass_len,
                   const unsigned char *salt, long salt_len,
                   long t_cost, long m_cost_kib, long parallelism,
                   long dklen, unsigned char *out);
int  jade_scrypt(const unsigned char *pass, long pass_len,
                 const unsigned char *salt, long salt_len,
                 long n, long r, long p, long dklen, unsigned char *out);

/* runtime/tls.c (requires OpenSSL) */
typedef struct jade_tls_conn jade_tls_conn;
void           jade_tls_init(void);
jade_tls_conn *jade_tls_connect(const char *host, int port);
long           jade_tls_send(jade_tls_conn *conn, const char *buf, long len);
long           jade_tls_recv(jade_tls_conn *conn, char *buf, long len);
void           jade_tls_close(jade_tls_conn *conn);
typedef struct jade_tls_listener jade_tls_listener;
jade_tls_listener *jade_tls_listen(const char *host, int port,
                                   const char *cert_path,
                                   const char *key_path);
jade_tls_conn *jade_tls_accept(jade_tls_listener *l);
void           jade_tls_listener_close(jade_tls_listener *l);
long           jade_tls_last_error(char *buf, long len);
long           jade_tls_peer_cert_subject(jade_tls_conn *conn, char *buf, long len);
long           jade_tls_protocol_version(jade_tls_conn *conn, char *buf, long len);
int            jade_dns_resolve(const char *host, char *out_buf, int out_len);
int            jade_dns_resolve_all(const char *host, char *out_buf, int out_len);

/* runtime/sqlite.c (requires sqlite3) */
void       *jade_sqlite_open(const char *path);
int         jade_sqlite_close(void *db);
int         jade_sqlite_exec(void *db, const char *sql);
const char *jade_sqlite_errmsg(void *db);
long        jade_sqlite_last_insert_id(void *db);
long        jade_sqlite_changes(void *db);
void       *jade_sqlite_prepare(void *db, const char *sql);
void        jade_sqlite_finalize(void *stmt);
int         jade_sqlite_reset(void *stmt);
int         jade_sqlite_bind_int(void *stmt, int idx, long val);
int         jade_sqlite_bind_float(void *stmt, int idx, double val);
int         jade_sqlite_bind_text(void *stmt, int idx, const char *val, long len);
int         jade_sqlite_bind_null(void *stmt, int idx);
int         jade_sqlite_bind_blob(void *stmt, int idx, const void *data, long len);
int         jade_sqlite_step(void *stmt);
int         jade_sqlite_column_count(void *stmt);
const char *jade_sqlite_column_name(void *stmt, int idx);
int         jade_sqlite_column_type(void *stmt, int idx);
long        jade_sqlite_column_int(void *stmt, int idx);
double      jade_sqlite_column_float(void *stmt, int idx);
const char *jade_sqlite_column_text(void *stmt, int idx);
long        jade_sqlite_column_text_len(void *stmt, int idx);
const void *jade_sqlite_column_blob(void *stmt, int idx);
long        jade_sqlite_column_blob_len(void *stmt, int idx);
int         jade_sqlite_begin(void *db);
int         jade_sqlite_commit(void *db);
int         jade_sqlite_rollback(void *db);

#ifdef __cplusplus
}
#endif
