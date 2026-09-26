//! Credentials for the three authorization scopes (constitution, additional
//! constraints; plan R7). The scopes are deliberately separate so the
//! frontend's read credential can never write or report a selection.
//!
//! * `catalog:read`  – the frontend (`X-Rosaray-Session`, ephemeral per start)
//! * `author`        – local authoring tools (`X-Rosaray-Author`)
//! * `system_one`    – the local System One component (`X-Rosaray-SystemOne`)

use std::path::Path;

use rand::RngCore;

pub const HEADER_CATALOG: &str = "X-Rosaray-Session";
pub const HEADER_AUTHOR: &str = "X-Rosaray-Author";
pub const HEADER_SYSTEM_ONE: &str = "X-Rosaray-SystemOne";

pub const DEFAULT_SOURCE_ID: &str = "system-one-local";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    CatalogRead,
    Author,
    SystemOne,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    /// Ephemeral: regenerated every start and handed to the frontend launcher.
    pub catalog: String,
    pub author: String,
    pub system_one: String,
    pub source_id: String,
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_or_create(path: &Path) -> std::io::Result<String> {
    if let Ok(t) = std::fs::read_to_string(path) {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    let token = random_token();
    std::fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

impl Credentials {
    /// Author and System One credentials persist in `dir` (owner-only files) so
    /// separate local components can read them; the catalog credential is new
    /// on every start.
    pub fn load_or_create(dir: &Path, catalog: String) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self {
            catalog,
            author: read_or_create(&dir.join("author.token"))?,
            system_one: read_or_create(&dir.join("system-one.token"))?,
            source_id: std::fs::read_to_string(dir.join("system-one.source_id"))
                .map(|s| s.trim().to_string())
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_SOURCE_ID.to_string()),
        })
    }

    /// For tests and embedding: fixed values, no files.
    pub fn fixed(catalog: &str, author: &str, system_one: &str) -> Self {
        Self { catalog: catalog.into(), author: author.into(), system_one: system_one.into(), source_id: DEFAULT_SOURCE_ID.into() }
    }

    /// Which scope (if any) does `header_name: value` grant? A credential only
    /// ever grants the scope of *its own* header.
    pub fn scope_of(&self, header_name: &str, value: &str) -> Option<Scope> {
        let (expected, scope) = match header_name {
            HEADER_CATALOG => (&self.catalog, Scope::CatalogRead),
            HEADER_AUTHOR => (&self.author, Scope::Author),
            HEADER_SYSTEM_ONE => (&self.system_one, Scope::SystemOne),
            _ => return None,
        };
        constant_time_eq(expected.as_bytes(), value.as_bytes()).then_some(scope)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// May a caller holding `have` perform an operation that needs `need`?
/// Reading the catalog is open to every scope; writes and exchange are not.
pub fn permits(have: Scope, need: Scope) -> bool {
    have == need || need == Scope::CatalogRead
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_credential_grants_only_its_own_scope() {
        let c = Credentials::fixed("cat", "auth", "so");
        assert_eq!(c.scope_of(HEADER_CATALOG, "cat"), Some(Scope::CatalogRead));
        assert_eq!(c.scope_of(HEADER_AUTHOR, "auth"), Some(Scope::Author));
        assert_eq!(c.scope_of(HEADER_SYSTEM_ONE, "so"), Some(Scope::SystemOne));
        // The frontend credential presented in another header grants nothing.
        assert_eq!(c.scope_of(HEADER_AUTHOR, "cat"), None);
        assert_eq!(c.scope_of(HEADER_SYSTEM_ONE, "cat"), None);
        assert_eq!(c.scope_of(HEADER_CATALOG, "wrong"), None);
    }

    #[test]
    fn writes_need_their_own_scope_but_reads_are_open() {
        assert!(permits(Scope::CatalogRead, Scope::CatalogRead));
        assert!(permits(Scope::Author, Scope::CatalogRead));
        assert!(permits(Scope::SystemOne, Scope::CatalogRead));
        assert!(!permits(Scope::CatalogRead, Scope::Author));
        assert!(!permits(Scope::CatalogRead, Scope::SystemOne));
        assert!(!permits(Scope::Author, Scope::SystemOne));
    }

    #[test]
    fn persisted_credentials_are_stable_and_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let a = Credentials::load_or_create(&dir.path().join("c"), "x".into()).unwrap();
        let b = Credentials::load_or_create(&dir.path().join("c"), "y".into()).unwrap();
        assert_eq!(a.author, b.author);
        assert_eq!(a.system_one, b.system_one);
        assert_ne!(a.catalog, b.catalog);
        assert_ne!(a.author, a.system_one);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("c/author.token")).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
    }
}
