use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::super::{Artifact, ExecContext, Inputs, NodeError, NodeExecutor, Outputs};

/// `builtin.image-source`: hands the chosen Image Asset's research
/// representation to the pipe.
pub struct ImageSource;

impl NodeExecutor for ImageSource {
    fn implementation_id(&self) -> &'static str {
        "builtin.image-source"
    }

    fn execute(&self, ctx: &ExecContext, _inputs: &Inputs, _params: &Map<String, Value>) -> Result<Outputs, NodeError> {
        let src = ctx
            .source
            .as_ref()
            .ok_or_else(|| NodeError::new("source_unavailable", "no image was supplied to the pipe"))?;
        let mut out = BTreeMap::new();
        out.insert("image".to_string(), Artifact::Image((**src).clone()));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::designer::runtime_adapter::ImageF32;
    use std::sync::Arc;

    #[test]
    fn passes_the_source_pixels_through_unchanged() {
        let img = ImageF32 { w: 2, h: 1, data: vec![3.0, 250.0] };
        let ctx = ExecContext { source: Some(Arc::new(img.clone())), ..Default::default() };
        let out = ImageSource.execute(&ctx, &Inputs::new(), &Map::new()).unwrap();
        assert_eq!(out["image"], Artifact::Image(img));
    }

    #[test]
    fn fails_clearly_without_a_source() {
        let err = ImageSource.execute(&ExecContext::default(), &Inputs::new(), &Map::new()).unwrap_err();
        assert_eq!(err.code, "source_unavailable");
    }
}
