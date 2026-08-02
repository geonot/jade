#include <sqlite3.h>
#include <stdlib.h>
#include <string.h>
#include "jinn_rt.h"
void *jinn_sqlite_open(const char *path) {
    sqlite3 *db = NULL;
    int rc = sqlite3_open(path, &db);
    if (rc != SQLITE_OK) {
        if (db) sqlite3_close(db);
        return NULL;
    }
    sqlite3_exec(db, "PRAGMA journal_mode=WAL;", NULL, NULL, NULL);
    return db;
}
int jinn_sqlite_close(void *db) {
    if (!db) return -1;
    return sqlite3_close((sqlite3 *)db) == SQLITE_OK ? 0 : -1;
}

int jinn_sqlite_exec(void *db, const char *sql) {
    if (!db || !sql) return -1;
    char *err = NULL;
    int rc = sqlite3_exec((sqlite3 *)db, sql, NULL, NULL, &err);
    if (err) sqlite3_free(err);
    return rc == SQLITE_OK ? 0 : -1;
}
const char *jinn_sqlite_errmsg(void *db) {
    if (!db) return "null database handle";
    return sqlite3_errmsg((sqlite3 *)db);
}

long jinn_sqlite_last_insert_id(void *db) {
    if (!db) return -1;
    return (long)sqlite3_last_insert_rowid((sqlite3 *)db);
}

long jinn_sqlite_changes(void *db) {
    if (!db) return 0;
    return (long)sqlite3_changes((sqlite3 *)db);
}

void *jinn_sqlite_prepare(void *db, const char *sql) {
    if (!db || !sql) return NULL;
    sqlite3_stmt *stmt = NULL;
    int rc = sqlite3_prepare_v2((sqlite3 *)db, sql, -1, &stmt, NULL);
    if (rc != SQLITE_OK) {
        if (stmt) sqlite3_finalize(stmt);
        return NULL;
    }
    return stmt;
}
void jinn_sqlite_finalize(void *stmt) {
    if (stmt) sqlite3_finalize((sqlite3_stmt *)stmt);
}
int jinn_sqlite_reset(void *stmt) {
    if (!stmt) return -1;
    sqlite3_clear_bindings((sqlite3_stmt *)stmt);
    return sqlite3_reset((sqlite3_stmt *)stmt) == SQLITE_OK ? 0 : -1;
}

int jinn_sqlite_bind_int(void *stmt, int idx, long val) {
    if (!stmt) return -1;
    return sqlite3_bind_int64((sqlite3_stmt *)stmt, idx, (sqlite3_int64)val) == SQLITE_OK ? 0 : -1;
}

int jinn_sqlite_bind_float(void *stmt, int idx, double val) {
    if (!stmt) return -1;
    return sqlite3_bind_double((sqlite3_stmt *)stmt, idx, val) == SQLITE_OK ? 0 : -1;
}
int jinn_sqlite_bind_text(void *stmt, int idx, const char *val, long len) {
    if (!stmt) return -1;
    return sqlite3_bind_text((sqlite3_stmt *)stmt, idx, val, (int)len, SQLITE_TRANSIENT) == SQLITE_OK ? 0 : -1;
}
int jinn_sqlite_bind_null(void *stmt, int idx) {
    if (!stmt) return -1;
    return sqlite3_bind_null((sqlite3_stmt *)stmt, idx) == SQLITE_OK ? 0 : -1;
}
int jinn_sqlite_bind_blob(void *stmt, int idx, const void *data, long len) {
    if (!stmt) return -1;
    return sqlite3_bind_blob((sqlite3_stmt *)stmt, idx, data, (int)len, SQLITE_TRANSIENT) == SQLITE_OK ? 0 : -1;
}

int jinn_sqlite_step(void *stmt) {
    if (!stmt) return -1;
    int rc = sqlite3_step((sqlite3_stmt *)stmt);
    if (rc == SQLITE_ROW) return 1;
    if (rc == SQLITE_DONE) return 0;
    return -1;
}
int jinn_sqlite_column_count(void *stmt) {
    if (!stmt) return 0;
    return sqlite3_column_count((sqlite3_stmt *)stmt);
}

const char *jinn_sqlite_column_name(void *stmt, int idx) {
    if (!stmt) return "";
    const char *name = sqlite3_column_name((sqlite3_stmt *)stmt, idx);
    return name ? name : "";
}

int jinn_sqlite_column_type(void *stmt, int idx) {
    if (!stmt) return 5;
    return sqlite3_column_type((sqlite3_stmt *)stmt, idx);
}

long jinn_sqlite_column_int(void *stmt, int idx) {
    if (!stmt) return 0;
    return (long)sqlite3_column_int64((sqlite3_stmt *)stmt, idx);
}

double jinn_sqlite_column_float(void *stmt, int idx) {
    if (!stmt) return 0.0;
    return sqlite3_column_double((sqlite3_stmt *)stmt, idx);
}

const char *jinn_sqlite_column_text(void *stmt, int idx) {
    if (!stmt) return "";
    const char *txt = (const char *)sqlite3_column_text((sqlite3_stmt *)stmt, idx);
    return txt ? txt : "";
}

long jinn_sqlite_column_text_len(void *stmt, int idx) {
    if (!stmt) return 0;
    return (long)sqlite3_column_bytes((sqlite3_stmt *)stmt, idx);
}

const void *jinn_sqlite_column_blob(void *stmt, int idx) {
    if (!stmt) return NULL;
    return sqlite3_column_blob((sqlite3_stmt *)stmt, idx);
}

long jinn_sqlite_column_blob_len(void *stmt, int idx) {
    if (!stmt) return 0;
    return (long)sqlite3_column_bytes((sqlite3_stmt *)stmt, idx);
}

int jinn_sqlite_begin(void *db) {
    return jinn_sqlite_exec(db, "BEGIN TRANSACTION");
}
int jinn_sqlite_commit(void *db) {
    return jinn_sqlite_exec(db, "COMMIT");
}
int jinn_sqlite_rollback(void *db) {
    return jinn_sqlite_exec(db, "ROLLBACK");
}
