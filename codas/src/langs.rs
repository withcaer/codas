//! Coda code generators.
//!
//! # Unstable
//!
//! The APIs exposed by this module are _primarily_
//! for use by automated tooling (macros, CLIs, etc.);
//! the exact APIs are subject to change, and may
//! not be well-optimized.

#[cfg(any(feature = "langs-duckdb", test))]
pub mod duckdb;

#[cfg(any(feature = "langs-open-api", test))]
pub mod open_api;

#[cfg(any(feature = "langs-python", test))]
pub mod python;

#[cfg(any(feature = "langs-rust", test))]
pub mod rust;

#[cfg(any(feature = "langs-sqlite", test))]
pub mod sqlite;

#[cfg(any(feature = "langs-typescript", test))]
pub mod typescript;
