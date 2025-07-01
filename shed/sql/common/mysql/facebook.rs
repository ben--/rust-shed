/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is dual-licensed under either the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree or the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree. You may select, at your option, one of the
 * above-listed licenses.
 */

//! This module is only used in internal Meta builds.

#![deny(warnings)]

mod fbmysql_wrapper;

pub use fbmysql_wrapper::Connection;
pub use fbmysql_wrapper::MysqlQueryTelemetry;
pub use fbmysql_wrapper::Transaction;
pub use mysql_client::MysqlError;
