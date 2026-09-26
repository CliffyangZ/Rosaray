use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::super::{input, require_f64, Artifact, ExecContext, ImageF32, Inputs, NodeError, NodeExecutor, Outputs};

/// `builtin.normalize`: percentile contrast stretch to 0–255.
pub struct Normalize;

pub fn normalize(img: &ImageF32, lo_pct: f64, hi_pct: f64) -> ImageF32 {
    let mut sorted = img.data.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len();
    let at = |pct: f64| sorted[((n as f64 * pct / 100.0).floor() as usize).min(n - 1)];
    let lo = at(lo_pct);
    let hi = sorted[(((n as f64 * hi_pct / 100.0).floor()) as usize).min(n - 1)];
    let k = 255.0 / f64::from(hi - lo).max(1e-6);
    ImageF32 {
        w: img.w,
        h: img.h,
        data: img.data.iter().map(|v| ((f64::from(*v) - f64::from(lo)) * k).clamp(0.0, 255.0) as f32).collect(),
    }
}

impl NodeExecutor for Normalize {
    fn implementation_id(&self) -> &'static str {
        "builtin.normalize"
    }

    fn execute(&self, _ctx: &ExecContext, inputs: &Inputs, params: &Map<String, Value>) -> Result<Outputs, NodeError> {
        let img = input(inputs, "image")?.as_image()?;
        if img.data.is_empty() {
            return Err(NodeError::new("empty_image", "the image has no pixels"));
        }
        let (lo, hi) = (require_f64(params, "lo")?, require_f64(params, "hi")?);
        let mut out = BTreeMap::new();
        out.insert("image".to_string(), Artifact::Image(normalize(img, lo, hi)));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::reference::*;
    use super::*;

    #[test]
    fn matches_the_javascript_on_the_fixed_image() {
        let r = load();
        let img = image(&r);
        for key in ["normalize", "normalize2"] {
            let got = normalize(&img, r[key]["lo"].as_f64().unwrap(), r[key]["hi"].as_f64().unwrap());
            assert_close(&got.data, &floats(&r[key]["out"]));
        }
    }

    #[test]
    fn a_flat_image_does_not_divide_by_zero() {
        let flat = ImageF32 { w: 2, h: 2, data: vec![9.0; 4] };
        assert!(normalize(&flat, 1.0, 99.0).data.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn missing_parameters_are_errors_not_defaults() {
        let inputs: Inputs = [("image".to_string(), std::sync::Arc::new(Artifact::Image(ImageF32 { w: 1, h: 1, data: vec![1.0] })))].into();
        let err = Normalize.execute(&ExecContext::default(), &inputs, &Map::new()).unwrap_err();
        assert_eq!(err.code, "parameter_missing");
    }
}
