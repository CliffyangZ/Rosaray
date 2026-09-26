//! Append-only hash-chain store (research §7, §13) shared by amendments and
//! verification records. Files are `NNNN-<uuid>.yaml`; each names the hash of
//! the previous file's bytes in `prev`. Existing files are never rewritten.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::contract::finding::{BundleRef, Finding, Severity, Subject};
use crate::bundle::model::{parse_yaml, to_yaml};

/// Serializes appends in this process so two writers cannot pick one `seq`.
static APPEND_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainRecord {
    pub schema: String,
    pub seq: u64,
    /// `b3:` hash of the previous file's bytes; `None` for the first record.
    pub prev: Option<String>,
    pub kind: String,
    pub subject: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub author: String,
    pub created_at: String,
    /// Type-specific fields (`original_evidence`, `affected_versions`, `suite`, …).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// What a caller supplies; the store fills `schema`, `seq`, `prev`, `created_at`.
#[derive(Debug, Clone)]
pub struct NewRecord {
    pub kind: String,
    pub subject: Value,
    pub reason: Option<String>,
    pub author: String,
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHead {
    pub seq: u64,
    /// The uuid in the record's file name (`NNNN-<uuid>.yaml`).
    pub record_id: String,
    /// `b3:` hash of the newest file's bytes.
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainBreak {
    pub seq: u64,
    pub file: String,
    pub reason: String,
}

impl ChainBreak {
    pub fn to_finding(&self, bundle: &BundleRef) -> Finding {
        Finding::build(
            Severity::Error,
            "chain_broken",
            bundle,
            Subject::file(self.file.clone()),
            format!("The history chain is broken at record {}: {}.", self.seq, self.reason),
            "Restore the original chain files from a backup; dependent versions stay unavailable until the chain verifies.",
        )
    }
}

#[derive(Debug, Clone)]
pub struct ChainRead {
    pub records: Vec<ChainRecord>,
    pub broken: Option<ChainBreak>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("the existing chain is broken and cannot be appended to: {0:?}")]
    Broken(ChainBreak),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not serialize record: {0}")]
    Serialize(String),
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("b3:{}", blake3::hash(bytes).to_hex())
}

fn chain_files(dir: &Path) -> std::io::Result<Vec<String>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".yaml") && n.split('-').next().is_some_and(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit())))
        .collect();
    names.sort();
    Ok(names)
}

/// Reads and verifies the chain in `dir`. Records up to a break are
/// returned; `broken` says where trust ends.
pub fn read_chain(dir: &Path) -> std::io::Result<ChainRead> {
    let mut records = Vec::new();
    let mut prev_hash: Option<String> = None;
    for (i, name) in chain_files(dir)?.into_iter().enumerate() {
        let expected_seq = i as u64 + 1;
        let bytes = fs::read(dir.join(&name))?;
        let brk = |reason: String| ChainBreak {
            seq: expected_seq,
            file: name.clone(),
            reason,
        };
        let record: ChainRecord = match std::str::from_utf8(&bytes)
            .map_err(|e| e.to_string())
            .and_then(parse_yaml::<ChainRecord>)
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(ChainRead { records, broken: Some(brk(format!("the record cannot be read ({e})"))) })
            }
        };
        if record.seq != expected_seq {
            return Ok(ChainRead {
                records,
                broken: Some(brk(format!("sequence number is {} but {} was expected", record.seq, expected_seq))),
            });
        }
        if record.prev != prev_hash {
            return Ok(ChainRead {
                records,
                broken: Some(brk("it does not link to the previous record's contents".to_string())),
            });
        }
        prev_hash = Some(hash_bytes(&bytes));
        records.push(record);
    }
    Ok(ChainRead { records, broken: None })
}

/// Appends a record and returns the new chain head. Refuses to extend a
/// broken chain; never rewrites an existing file.
pub fn append(dir: &Path, new: NewRecord) -> Result<ChainHead, ChainError> {
    let _guard = APPEND_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    fs::create_dir_all(dir)?;
    let existing = read_chain(dir)?;
    if let Some(b) = existing.broken {
        return Err(ChainError::Broken(b));
    }
    let names = chain_files(dir)?;
    let prev = match names.last() {
        Some(last) => Some(hash_bytes(&fs::read(dir.join(last))?)),
        None => None,
    };
    let seq = names.len() as u64 + 1;
    let record = ChainRecord {
        schema: "quantify-kb/1".to_string(),
        seq,
        prev,
        kind: new.kind,
        subject: new.subject,
        reason: new.reason,
        author: new.author,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        extra: new.extra,
    };
    let yaml = to_yaml(&record).map_err(ChainError::Serialize)?;
    let record_id = Uuid::new_v4().to_string();
    let path = dir.join(format!("{seq:04}-{record_id}.yaml"));
    let mut f = OpenOptions::new().write(true).create_new(true).open(&path)?;
    f.write_all(yaml.as_bytes())?;
    f.sync_all()?;
    Ok(ChainHead {
        seq,
        record_id,
        hash: hash_bytes(yaml.as_bytes()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: &str) -> NewRecord {
        NewRecord {
            kind: kind.into(),
            subject: serde_json::json!({"node_id": "rosaray.threshold"}),
            reason: Some("because".into()),
            author: "researcher-local".into(),
            extra: Default::default(),
        }
    }

    #[test]
    fn append_links_records_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let h1 = append(dir.path(), rec("passed")).unwrap();
        let h2 = append(dir.path(), rec("withdrawn")).unwrap();
        assert_eq!((h1.seq, h2.seq), (1, 2));
        let read = read_chain(dir.path()).unwrap();
        assert!(read.broken.is_none());
        assert_eq!(read.records.len(), 2);
        assert_eq!(read.records[0].prev, None);
        assert_eq!(read.records[1].prev, Some(h1.hash));
    }

    #[test]
    fn tampering_with_an_earlier_record_breaks_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), rec("passed")).unwrap();
        append(dir.path(), rec("failed")).unwrap();
        let first = chain_files(dir.path()).unwrap().remove(0);
        let p = dir.path().join(&first);
        let text = fs::read_to_string(&p).unwrap().replace("because", "tampered");
        fs::write(&p, text).unwrap();
        let read = read_chain(dir.path()).unwrap();
        let brk = read.broken.expect("tamper detected");
        assert_eq!(brk.seq, 2);
        assert!(matches!(append(dir.path(), rec("passed")), Err(ChainError::Broken(_))));
        let f = brk.to_finding(&BundleRef::new("x.y", None));
        assert_eq!(f.code, "chain_broken");
    }

    #[test]
    fn deleting_a_middle_record_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        for k in ["passed", "failed", "passed"] {
            append(dir.path(), rec(k)).unwrap();
        }
        let names = chain_files(dir.path()).unwrap();
        fs::remove_file(dir.path().join(&names[1])).unwrap();
        assert!(read_chain(dir.path()).unwrap().broken.is_some());
    }

    #[test]
    fn empty_directory_is_an_empty_valid_chain() {
        let dir = tempfile::tempdir().unwrap();
        let read = read_chain(&dir.path().join("none")).unwrap();
        assert!(read.records.is_empty() && read.broken.is_none());
    }
}
