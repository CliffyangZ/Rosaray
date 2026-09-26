//! Quantify KB: file-based, UI-independent knowledge base (no HTTP types).
pub mod bundle;
pub mod catalog;
pub mod eligibility;
pub mod evidence;
pub mod exchange;
pub mod identity;
pub mod inspector;
pub mod profile;
pub mod publish;
pub mod seed;
pub mod trust;

/// Folders of the Knowledge Base tree (contracts/bundle-format.md §Layout).
pub const LAYOUT_DIRS: &[&str] = &[
    "nodes",
    "pipes",
    "drafts/nodes",
    "drafts/pipes",
    "amendments",
    "verification",
];

/// Creates the Knowledge Base folder tree if missing; existing content is
/// never touched.
pub fn ensure_layout(root: &std::path::Path) -> std::io::Result<()> {
    for dir in LAYOUT_DIRS {
        std::fs::create_dir_all(root.join(dir))?;
    }
    Ok(())
}
