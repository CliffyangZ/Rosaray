//! Rosaray QKB: the executable Quantitative Knowledge Base and the System One
//! selection protocol. No HTTP types live here; `rosaray-service` only adds
//! transport and authorization on top (constitution I, plan R1).
pub mod auth;
pub mod author;
pub mod bundle;
pub mod catalog;
pub mod cleanup;
pub mod contract;
pub mod eligibility;
pub mod executability;
pub mod event_repo;
pub mod evidence;
pub mod graph_identity;
pub mod guard;
pub mod identity;
pub mod profile;
pub mod runtime_adapter;
pub mod seed;
pub mod store;
pub mod system_one;
pub mod submit;
pub mod trust;

/// Folders of the Knowledge Base tree. There is no `drafts/` folder: nothing
/// unexecutable is ever retained (constitution III).
pub const LAYOUT_DIRS: &[&str] = &["nodes", "pipes", "amendments", "verification"];

/// Creates the Knowledge Base folder tree if missing; existing content is
/// never touched.
pub fn ensure_layout(root: &std::path::Path) -> std::io::Result<()> {
    for dir in LAYOUT_DIRS {
        std::fs::create_dir_all(root.join(dir))?;
    }
    Ok(())
}
