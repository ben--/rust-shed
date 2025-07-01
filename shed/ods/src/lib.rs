/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is dual-licensed under either the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree or the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree. You may select, at your option, one of the
 * above-listed licenses.
 */

//! This is a mock for multiplatform/ods library.

#![deny(warnings, missing_docs, clippy::all, rustdoc::broken_intra_doc_links)]

pub mod oss;

#[cfg(fbcode_build)]
pub use fb_ods::send_data_to_ods;
#[cfg(not(fbcode_build))]
pub use oss::send_data_to_ods;
