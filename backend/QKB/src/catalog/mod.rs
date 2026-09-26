//! Derived catalog: scan → SQLite index → filtered query. Everything here is
//! rebuildable from bundle files (FR-040); nothing in it is authoritative.
pub mod query;
pub mod repo;
pub mod scan;
pub mod status;
