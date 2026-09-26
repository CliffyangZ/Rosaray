use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::super::{input, require_f64, require_str, Artifact, ExecContext, ImageF32, Inputs, MaskU8, NodeError, NodeExecutor, Outputs};

/// `builtin.threshold`: global threshold (manual or Otsu), optionally inverted.
pub struct Threshold;

/// Otsu's method on a 256-bin histogram, exactly as the prototype computes it
/// (bins by truncation toward zero, clamped to 0–255).
pub fn otsu(d: &[f32]) -> f64 {
    let mut hist = [0f64; 256];
    for v in d {
        hist[(*v as i32).clamp(0, 255) as usize] += 1.0;
    }
    let tot = d.len() as f64;
    let sum: f64 = (0..256).map(|i| i as f64 * hist[i]).sum();
    let (mut sb, mut wb, mut best, mut th) = (0f64, 0f64, 0f64, 0f64);
    for i in 0..256 {
        wb += hist[i];
        if wb == 0.0 {
            continue;
        }
        let wf = tot - wb;
        if wf == 0.0 {
            break;
        }
        sb += i as f64 * hist[i];
        let (mb, mf) = (sb / wb, (sum - sb) / wf);
        let v = wb * wf * (mb - mf).powi(2);
        if v > best {
            best = v;
            th = i as f64;
        }
    }
    th
}

pub fn threshold(img: &ImageF32, th: f64, invert: bool) -> MaskU8 {
    MaskU8 { w: img.w, h: img.h, data: img.data.iter().map(|v| u8::from((f64::from(*v) > th) != invert)).collect() }
}

impl NodeExecutor for Threshold {
    fn implementation_id(&self) -> &'static str {
        "builtin.threshold"
    }

    fn execute(&self, _ctx: &ExecContext, inputs: &Inputs, params: &Map<String, Value>) -> Result<Outputs, NodeError> {
        let img = input(inputs, "image")?.as_image()?;
        let th = match require_str(params, "mode")? {
            "otsu" => otsu(&img.data),
            "manual" => require_f64(params, "value")?,
            other => return Err(NodeError::new("parameter_invalid", format!("unknown threshold mode \"{other}\""))),
        };
        let invert = params.get("invert").and_then(Value::as_bool).unwrap_or(false);
        let mut out = BTreeMap::new();
        out.insert("mask".to_string(), Artifact::Mask(threshold(img, th, invert)));
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
        let th = otsu(&img.data);
        assert_eq!(th, r["threshold_otsu"]["th"].as_f64().unwrap());
        assert_eq!(threshold(&img, th, false), mask(&r, &r["threshold_otsu"]["mask"]));
        assert_eq!(threshold(&img, 120.0, false), mask(&r, &r["threshold_manual"]["mask"]));
        assert_eq!(threshold(&img, 120.0, true), mask(&r, &r["threshold_invert"]["mask"]));
    }

    #[test]
    fn an_unknown_mode_is_an_error() {
        let inputs: Inputs = [("image".to_string(), std::sync::Arc::new(Artifact::Image(ImageF32 { w: 1, h: 1, data: vec![1.0] })))].into();
        let params: Map<String, Value> = serde_json::from_str(r#"{"mode":"sobel"}"#).unwrap();
        assert_eq!(Threshold.execute(&ExecContext::default(), &inputs, &params).unwrap_err().code, "parameter_invalid");
    }
}
