/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is dual-licensed under either the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree or the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree. You may select, at your option, one of the
 * above-listed licenses.
 */

//! Module containing sqlite related structures and traits

#![allow(clippy::mutex_atomic)]

use std::ops::Deref;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use rusqlite::Connection as SqliteConnection;

/// Lock to ensure that only one connection is in use for writes at a time
/// inside the process TODO: Remove this lock, and replace by better connection
/// handling (as SQLite will get this right if we use a single connection to
/// each file). See T59837828
static CONN_LOCK: Mutex<bool> = Mutex::new(true);

static CONN_CONDVAR: Condvar = Condvar::new();

impl crate::Connection {
    /// Given a `rusqlite::Connection` create a connection to Sqlite database that might be used
    /// by this crate.
    pub fn with_sqlite(con: SqliteConnection) -> Self {
        SqliteMultithreaded::new(con).into()
    }

    /// Given a `rusqlite::Connection` create a connection to Sqlite database that might be used
    /// by this crate, and add callbacks for when operations happen.
    pub fn with_sqlite_callbacks(
        con: SqliteConnection,
        callbacks: Box<dyn SqliteCallbacks>,
    ) -> Self {
        SqliteMultithreaded::new_with_callbacks(con, callbacks).into()
    }

    /// Given a `rusqlite::Connection` create a connection to Sqlite database that might be used
    /// by this crate, plus a function that returns the HLC from the replica
    /// that serves reads.
    pub fn with_sqlite_hlc_provider_and_callbacks(
        con: SqliteConnection,
        hlc_provider: Arc<Box<SqliteHlcProvider>>,
        callbacks: Box<dyn SqliteCallbacks>,
    ) -> Self {
        SqliteMultithreaded::new_with_sqlite_hlc_provider_and_callbacks(
            con,
            hlc_provider,
            callbacks,
        )
        .into()
    }
}

/// Sqlite query categorization to allow callbacks to perform different
/// things on the basis of the operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SqliteQueryType {
    /// The caller is starting a query that will read from the database.
    Read,

    /// The caller is starting a query that will write to the database.
    Write,

    /// The caller is starting a query that will modify the database schema
    /// (e.g. CREATE TABLE or ALTER TABLE).
    SchemaChange,

    /// The caller is starting a transaction.  The `after_transaction_commit`
    /// callback will be called if the transaction is committed.
    Transaction,
}

/// Callbacks for sqlite operations.  These are used to customize behavior or
/// track operations.
#[async_trait]
pub trait SqliteCallbacks: Send + Sync {
    /// Called when the sqlite connection guard is acquired for a query.
    async fn query_start(&self, _query_type: SqliteQueryType) -> Result<()> {
        Ok(())
    }

    /// Called when a transaction has been committed and the sqlite connection
    /// guard has been released.
    async fn after_transaction_commit(&self) {}
}

/// Callback to provide the HLC from the last update to the DB. Used in tests
pub type SqliteHlcProvider = dyn Fn() -> i64 + Send + Sync;

/// Wrapper around rusqlite connection that makes it fully thread safe (but not deadlock safe)
#[derive(Clone)]
pub struct SqliteMultithreaded {
    inner: Arc<SqliteMultithreadedInner>,
    hlc_provider: Option<Arc<Box<SqliteHlcProvider>>>,
}

/// Shared inner part of SqliteMultithreded plus any active connection guard.
pub struct SqliteMultithreadedInner {
    connection: Mutex<Option<SqliteConnection>>,
    condvar: Condvar,
    callbacks: Option<Box<dyn SqliteCallbacks>>,
}

/// Guard containing an active connection.
///
/// When this guard is destroyed, the connection is put back and threads that
/// are waiting for it are notified.
pub struct SqliteConnectionGuard {
    inner: Arc<SqliteMultithreadedInner>,
    // drop() needs to remove the connection, so use Option<...> here
    connection: Option<SqliteConnection>,
}

impl SqliteConnectionGuard {
    fn new(inner: Arc<SqliteMultithreadedInner>) -> SqliteConnectionGuard {
        let _global_lock =
            CONN_CONDVAR.wait_while(CONN_LOCK.lock().expect("lock poisoned"), |allowed| {
                if *allowed {
                    *allowed = false;
                    false
                } else {
                    true
                }
            });
        let connection = {
            let mut connection = inner
                .condvar
                .wait_while(inner.connection.lock().expect("poisoned lock"), |con| {
                    con.is_none()
                })
                .expect("poisoned lock");

            connection.take().expect("connection should not be empty")
        };

        SqliteConnectionGuard {
            inner,
            connection: Some(connection),
        }
    }

    /// Commit a transaction that is being executed on this connection, and
    /// then release the connection.  If the commit fails, the connection is
    /// not release, and is instead returned along with the error.
    // TODO: Remove allow
    #[allow(clippy::result_large_err)]
    pub async fn commit(self) -> Result<(), (Self, rusqlite::Error)> {
        fn commit_and_release(
            guard: SqliteConnectionGuard,
        ) -> Result<Arc<SqliteMultithreadedInner>, (SqliteConnectionGuard, rusqlite::Error)>
        {
            match guard.execute_batch("COMMIT") {
                Ok(()) => Ok(guard.inner.clone()),
                Err(e) => Err((guard, e)),
            }
        }

        let inner = commit_and_release(self)?;
        if let Some(callbacks) = &inner.callbacks {
            callbacks.after_transaction_commit().await;
        }
        Ok(())
    }
}

impl Deref for SqliteConnectionGuard {
    type Target = SqliteConnection;

    fn deref(&self) -> &Self::Target {
        self.connection
            .as_ref()
            .expect("invariant violation - deref called after drop()")
    }
}

impl Drop for SqliteConnectionGuard {
    fn drop(&mut self) {
        *(CONN_LOCK.lock().expect("lock poisoned")) = true;
        let mut connection = self.inner.connection.lock().expect("poisoned lock");
        connection.get_or_insert(self.connection.take().unwrap());
        // notify others that wait for this connection
        self.inner.condvar.notify_one();
        CONN_CONDVAR.notify_one();
    }
}

impl SqliteMultithreaded {
    /// Create a new instance wrapping the provided sqlite connection.
    pub fn new(connection: SqliteConnection) -> Self {
        Self {
            inner: Arc::new(SqliteMultithreadedInner {
                connection: Mutex::new(Some(connection)),
                condvar: Condvar::new(),
                callbacks: None,
            }),
            hlc_provider: None,
        }
    }

    /// Create a new instance wrapping the provided sqlite connection, and
    /// with callbacks that are called when sqlite operations happen.
    pub fn new_with_callbacks(
        connection: SqliteConnection,
        callbacks: Box<dyn SqliteCallbacks>,
    ) -> Self {
        Self {
            inner: Arc::new(SqliteMultithreadedInner {
                connection: Mutex::new(Some(connection)),
                condvar: Condvar::new(),
                callbacks: Some(callbacks),
            }),
            hlc_provider: None,
        }
    }

    /// Create a new instance wrapping the provided sqlite connection, and
    /// with callbacks that are called when sqlite operations happen.
    pub fn new_with_sqlite_hlc_provider_and_callbacks(
        connection: SqliteConnection,
        hlc_provider: Arc<Box<SqliteHlcProvider>>,
        callbacks: Box<dyn SqliteCallbacks>,
    ) -> Self {
        Self {
            inner: Arc::new(SqliteMultithreadedInner {
                connection: Mutex::new(Some(connection)),
                condvar: Condvar::new(),
                callbacks: Some(callbacks),
            }),
            hlc_provider: Some(hlc_provider),
        }
    }

    /// Returns a guard that acquires the sqlite connection.
    ///
    /// When guard is destroyed then connection is put back and threads that are waiting for it
    /// are notified.
    ///
    /// NOTE: This is a lock which will block any other `acquire_sqlite_connection()` calls, so
    /// you must not hold this over an await point as this may cause a deadlock.
    pub async fn acquire_sqlite_connection(
        &self,
        query_type: SqliteQueryType,
    ) -> Result<SqliteConnectionGuard> {
        if let Some(callbacks) = &self.inner.callbacks {
            callbacks.query_start(query_type).await?;
        }
        Ok(SqliteConnectionGuard::new(self.inner.clone()))
    }

    /// Get the timestamp of the last write to the database.
    /// Used only in tests.
    pub fn hlc_ts_lower_bound(&self) -> Option<i64> {
        self.hlc_provider.clone().map(|prov| prov())
    }
}
/// Query Telemetry for Sqlite queries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteQueryTelemetry {
    /// Timestamp of the last update to the database
    pub hlc_ts_lower_bound: i64,
}

impl SqliteQueryTelemetry {
    /// Create a new instance of SqliteQueryTelemetry
    pub fn new(hlc_ts_lower_bound: i64) -> Self {
        Self { hlc_ts_lower_bound }
    }
}
