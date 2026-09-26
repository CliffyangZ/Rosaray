//! Domain pipe rules (FR-046, FR-004). The core validator never branches on a
//! domain; it only calls the rules registered for the pipe's `domain`. The
//! dental-image rule set lives here; another domain simply registers others.

use super::finding::{BundleRef, Finding, Severity, Subject};
use crate::kb::bundle::model::GraphFile;

pub trait DomainRule {
    fn check(&self, bundle: &BundleRef, graph: &GraphFile) -> Vec<Finding>;
}

/// Exactly one `image-source` per dental-image pipe.
pub struct SingleImageSource;

pub const IMAGE_SOURCE_ID: &str = "rosaray.image-source";

impl DomainRule for SingleImageSource {
    fn check(&self, bundle: &BundleRef, graph: &GraphFile) -> Vec<Finding> {
        let sources: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.node_ref.id == IMAGE_SOURCE_ID)
            .map(|n| n.instance_id.as_str())
            .collect();
        if sources.len() == 1 {
            return Vec::new();
        }
        let (explanation, action) = if sources.is_empty() {
            (
                "The pipe has no image source, so nothing supplies the image.".to_string(),
                "Add exactly one Image source node.",
            )
        } else {
            (
                format!("The pipe has {} image sources ({}); a dental-image pipe takes exactly one.", sources.len(), sources.join(", ")),
                "Remove all but one Image source node.",
            )
        };
        vec![Finding::build(
            Severity::Error,
            "source_count",
            bundle,
            Subject::bundle(bundle.id.clone()),
            explanation,
            action,
        )]
    }
}

/// The rules registered for `domain`. Unknown or absent domains have none.
pub fn rules_for(domain: Option<&str>) -> Vec<Box<dyn DomainRule>> {
    match domain {
        Some("dental-image") => vec![Box::new(SingleImageSource)],
        _ => Vec::new(),
    }
}
