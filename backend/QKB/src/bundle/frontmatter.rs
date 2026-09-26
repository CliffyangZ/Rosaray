//! Markdown frontmatter splitter and typed bundle header (research §2).

use serde::{Deserialize, Serialize};

use super::model::{parse_yaml, Kind};

pub const SCHEMA_PREFIX: &str = "quantify-kb/";
pub const SUPPORTED_MAJOR: u32 = 1;

/// The common header of `ALGONODE.md` / `ALGOPIPE.md`. Everything except
/// what identifies the bundle is optional at parse time; validators decide
/// what a draft or a publication requires.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Header {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub intended_use: Option<String>,
    #[serde(default)]
    pub limitations: Option<String>,
    #[serde(default)]
    pub research_use_only: Option<bool>,
    #[serde(default)]
    pub release_kind: Option<String>,
    /// `paper_derived` for bundles made from a reviewed paper candidate (FR-027).
    #[serde(default)]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct FrontmatterError {
    /// Finding code the reader reports this under.
    pub code: &'static str,
    pub message: String,
}

impl FrontmatterError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Splits `---\n<yaml>\n---\n<body>` into `(yaml, body)`.
pub fn split_frontmatter(text: &str) -> Result<(&str, &str), FrontmatterError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or_else(|| FrontmatterError::new("frontmatter_invalid", "the file does not start with a `---` frontmatter block"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let yaml = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Ok((yaml, body));
        }
        offset += line.len();
    }
    Err(FrontmatterError::new("frontmatter_invalid", "the frontmatter block is never closed with `---`"))
}

/// Rejects an unknown schema major (`unsupported_schema`).
pub fn check_schema(schema: &str) -> Result<(), FrontmatterError> {
    let unsupported = || {
        FrontmatterError::new(
            "unsupported_schema",
            format!("schema \"{schema}\" is not supported (this version reads {SCHEMA_PREFIX}{SUPPORTED_MAJOR})"),
        )
    };
    let major = schema.strip_prefix(SCHEMA_PREFIX).ok_or_else(unsupported)?;
    let major: u32 = major.split('.').next().unwrap_or("").parse().map_err(|_| unsupported())?;
    if major == SUPPORTED_MAJOR {
        Ok(())
    } else {
        Err(unsupported())
    }
}

/// Parses the header and returns it with the Markdown body.
pub fn parse_header(text: &str) -> Result<(Header, String), FrontmatterError> {
    let (yaml, body) = split_frontmatter(text)?;
    let header: Header = parse_yaml(yaml).map_err(|e| FrontmatterError::new("frontmatter_invalid", e))?;
    check_schema(&header.schema)?;
    Ok((header, body.to_string()))
}

/// Renders a scalar for a YAML `key: value` line (strings are double-quoted
/// JSON, which is valid YAML).
fn yaml_scalar(value: &serde_json::Value) -> String {
    value.to_string()
}

/// Sets a top-level scalar field in the frontmatter block *textually*, so the
/// rest of a hand-edited header (ordering, comments) is left exactly as the
/// author wrote it. Adds the field before the closing `---` if absent.
pub fn set_field(text: &str, key: &str, value: &serde_json::Value) -> Result<String, FrontmatterError> {
    let (yaml, body) = split_frontmatter(text)?;
    let line = format!("{key}: {}", yaml_scalar(value));
    let mut replaced = false;
    let mut lines: Vec<String> = yaml
        .lines()
        .map(|l| {
            let is_key = l.strip_prefix(key).is_some_and(|rest| rest.starts_with(':'));
            if is_key && !replaced {
                replaced = true;
                line.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !replaced {
        lines.push(line);
    }
    Ok(format!("---\n{}\n---\n{}", lines.join("\n"), body))
}

/// Replaces the Markdown body, keeping the frontmatter block untouched.
pub fn replace_body(text: &str, new_body: &str) -> Result<String, FrontmatterError> {
    let (yaml, _) = split_frontmatter(text)?;
    let yaml = yaml.trim_end_matches(['\r', '\n']);
    Ok(format!("---\n{yaml}\n---\n{new_body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "---\nschema: quantify-kb/1\nkind: algonode\nid: rosaray.threshold\nversion: 1.0.0\nname: Threshold\nresearch_use_only: true\n---\nBody text.\n";

    #[test]
    fn splits_and_parses_header() {
        let (h, body) = parse_header(DOC).unwrap();
        assert_eq!(h.id, "rosaray.threshold");
        assert_eq!(h.kind, Some(Kind::Algonode));
        assert_eq!(h.research_use_only, Some(true));
        assert_eq!(body, "Body text.\n");
    }

    #[test]
    fn rejects_unknown_major_schema() {
        let doc = DOC.replace("quantify-kb/1", "quantify-kb/2");
        assert_eq!(parse_header(&doc).unwrap_err().code, "unsupported_schema");
        let doc = DOC.replace("quantify-kb/1", "other/1");
        assert_eq!(parse_header(&doc).unwrap_err().code, "unsupported_schema");
    }

    #[test]
    fn rejects_missing_or_unclosed_frontmatter() {
        assert_eq!(parse_header("no frontmatter").unwrap_err().code, "frontmatter_invalid");
        assert_eq!(parse_header("---\nid: x\n").unwrap_err().code, "frontmatter_invalid");
    }

    #[test]
    fn set_field_replaces_or_appends_without_disturbing_the_rest() {
        let out = set_field(DOC, "version", &serde_json::json!("2.0.0")).unwrap();
        let (h, body) = parse_header(&out).unwrap();
        assert_eq!(h.version, "2.0.0");
        assert_eq!(h.id, "rosaray.threshold");
        assert_eq!(body, "Body text.\n");
        let out = set_field(DOC, "status", &serde_json::json!("published")).unwrap();
        assert_eq!(parse_header(&out).unwrap().0.status.as_deref(), Some("published"));
    }

    #[test]
    fn replace_body_keeps_header() {
        let out = replace_body(DOC, "New.\n").unwrap();
        let (h, body) = parse_header(&out).unwrap();
        assert_eq!(h.name.as_deref(), Some("Threshold"));
        assert_eq!(body, "New.\n");
    }

    #[test]
    fn handles_crlf() {
        let doc = DOC.replace('\n', "\r\n");
        assert!(parse_header(&doc).is_ok());
    }
}
