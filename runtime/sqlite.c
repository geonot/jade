/* runtime/sqlite.c — SQLite3 wrappers for Jade
 *
 * Provides a safe, opaque interface to SQLite3.
 * All strings are C strings (null-terminated).
 * Parameterized queries prevent SQL injection.
 * Linked with -lsqlite3.
 */
#include <sqlite3.h>
#include <stdlib.h>
#include <string.h>

/* ── Database handle ─────────────────────────────────────── */

/* Open a database. Returns handle pointer, or NULL on failure. */
void *jade_sqlite_open(const char *path) {
    sqlite3 *db = NULL;
    int rc = sqlite3_open(path, &db);
    if (rc != SQLITE_OK) {
        if (db) sqlite3_close(db);
        return NULL;
    }
    /* Enable WAL mode for better concurrency */
    sqlite3_exec(db, "PRAGMA journal_mode=WAL;", NULL, NULL, NULL);
    return db;
}

/* Close a database. Returns 0 on success. */
int jade_sqlite_close(void *db) {
    if (!db) return -1;
    return sqlite3_close((sqlite3 *)db) == SQLITE_OK ? 0 : -1;
}

/* Execute SQL that returns no rows (CREATE, INSERT, UPDATE, DELETE).
 * Returns 0 on success, -1 on error. */
int jade_sqlite_exec(void *db, const char *sql) {
    if (!db || !sql) return -1;
    char *err = NULL;
    int rc = sqlite3_exec((sqlite3 *)db, sql, NULL, NULL, &err);
    if (err) sqlite3_free(err);
    return rc == SQLITE_OK ? 0 : -1;
}

/* Get the last error message. Returns pointer to internal string. */
const char *jade_sqlite_errmsg(void *db) {
    if (!db) return "null database handle";
    return sqlite3_errmsg((sqlite3 *)db);
}

/* Get the rowid of the last INSERT. */
long jade_sqlite_last_insert_id(void *db) {
    if (!db) return -1;
    return (long)sqlite3_last_insert_rowid((sqlite3 *)db);
}

/* Get the number of rows changed by last INSERT/UPDATE/DELETE. */
long jade_sqlite_changes(void *db) {
    if (!db) return 0;
    return (long)sqlite3_changes((sqlite3 *)db);
}

/* ── Prepared statements ─────────────────────────────────── */

/* Prepare a SQL statement. Returns statement handle, or NULL on error. */
void *jade_sqlite_prepare(void *db, const char *sql) {
    if (!db || !sql) return NULL;
    sqlite3_stmt *stmt = NULL;
    int rc = sqlite3_prepare_v2((sqlite3 *)db, sql, -1, &stmt, NULL);
    if (rc != SQLITE_OK) {
        if (stmt) sqlite3_finalize(stmt);
        return NULL;
    }
    return stmt;
}

/* Finalize (free) a prepared statement. */
void jade_sqlite_finalize(void *stmt) {
    if (stmt) sqlite3_finalize((sqlite3_stmt *)stmt);
}

/* Reset a statement for re-execution with new bindings. */
int jade_sqlite_reset(void *stmt) {
    if (!stmt) return -1;
    sqlite3_clear_bindings((sqlite3_stmt *)stmt);
    return sqlite3_reset((sqlite3_stmt *)stmt) == SQLITE_OK ? 0 : -1;
}

/* ── Binding parameters (1-indexed) ──────────────────────── */

int jade_sqlite_bind_int(void *stmt, int idx, long val) {
    if (!stmt) return -1;
    return sqlite3_bind_int64((sqlite3_stmt *)stmt, idx, (sqlite3_int64)val) == SQLITE_OK ? 0 : -1;
}

int jade_sqlite_bind_float(void *stmt, int idx, double val) {
    if (!stmt) return -1;
    return sqlite3_bind_double((sqlite3_stmt *)stmt, idx, val) == SQLITE_OK ? 0 : -1;
}

int jade_sqlite_bind_text(void *stmt, int idx, const char *val, long len) {
    if (!stmt) return -1;
    return sqlite3_bind_text((sqlite3_stmt *)stmt, idx, val, (int)len, SQLITE_TRANSIENT) == SQLITE_OK ? 0 : -1;
}

int jade_sqlite_bind_null(void *stmt, int idx) {
    if (!stmt) return -1;
    return sqlite3_bind_null((sqlite3_stmt *)stmt, idx) == SQLITE_OK ? 0 : -1;
}

int jade_sqlite_bind_blob(void *stmt, int idx, const void *data, long len) {
    if (!stmt) return -1;
    return sqlite3_bind_blob((sqlite3_stmt *)stmt, idx, data, (int)len, SQLITE_TRANSIENT) == SQLITE_OK ? 0 : -1;
}

/* ── Stepping and reading columns ────────────────────────── */

/* Step the statement. Returns 1 if a row is available, 0 if done, -1 on error. */
int jade_sqlite_step(void *stmt) {
    if (!stmt) return -1;
    int rc = sqlite3_step((sqlite3_stmt *)stmt);
    if (rc == SQLITE_ROW) return 1;
    if (rc == SQLITE_DONE) return 0;
    return -1;
}

/* Get the number of columns in the result set. */
int jade_sqlite_column_count(void *stmt) {
    if (!stmt) return 0;
    return sqlite3_column_count((sqlite3_stmt *)stmt);
}

/* Get the column name (0-indexed). */
const char *jade_sqlite_column_name(void *stmt, int idx) {
    if (!stmt) return "";
    const char *name = sqlite3_column_name((sqlite3_stmt *)stmt, idx);
    return name ? name : "";
}

/* Get column type: 1=INTEGER, 2=FLOAT, 3=TEXT, 4=BLOB, 5=NULL */
int jade_sqlite_column_type(void *stmt, int idx) {
    if (!stmt) return 5;
    return sqlite3_column_type((sqlite3_stmt *)stmt, idx);
}

/* Read an integer column. */
long jade_sqlite_column_int(void *stmt, int idx) {
    if (!stmt) return 0;
    return (long)sqlite3_column_int64((sqlite3_stmt *)stmt, idx);
}

/* Read a float column. */
double jade_sqlite_column_float(void *stmt, int idx) {
    if (!stmt) return 0.0;
    return sqlite3_column_double((sqlite3_stmt *)stmt, idx);
}

/* Read a text column. Returns pointer to internal string (valid until next step/finalize). */
const char *jade_sqlite_column_text(void *stmt, int idx) {
    if (!stmt) return "";
    const char *txt = (const char *)sqlite3_column_text((sqlite3_stmt *)stmt, idx);
    return txt ? txt : "";
}

/* Read text column length. */
long jade_sqlite_column_text_len(void *stmt, int idx) {
    if (!stmt) return 0;
    return (long)sqlite3_column_bytes((sqlite3_stmt *)stmt, idx);
}

/* Read blob column data. */
const void *jade_sqlite_column_blob(void *stmt, int idx) {
    if (!stmt) return NULL;
    return sqlite3_column_blob((sqlite3_stmt *)stmt, idx);
}

/* Read blob column length. */
long jade_sqlite_column_blob_len(void *stmt, int idx) {
    if (!stmt) return 0;
    return (long)sqlite3_column_bytes((sqlite3_stmt *)stmt, idx);
}

/* ── Transactions ────────────────────────────────────────── */

int jade_sqlite_begin(void *db) {
    return jade_sqlite_exec(db, "BEGIN TRANSACTION");
}

int jade_sqlite_commit(void *db) {
    return jade_sqlite_exec(db, "COMMIT");
}

int jade_sqlite_rollback(void *db) {
    return jade_sqlite_exec(db, "ROLLBACK");
}
