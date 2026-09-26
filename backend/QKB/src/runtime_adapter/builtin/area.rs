use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::super::{input, Artifact, ExecContext, Inputs, MaskU8, NodeError, NodeExecutor, Outputs};

/// `builtin.area`: pixel count, physical area (mm²) and 4-connected component
/// count of a mask. Physical area needs the image's pixel spacing; without it
/// the node fails with a clear error rather than assuming a calibration.
pub struct Area;

/// 4-connected components, as the prototype counts them.
pub fn components(m: &MaskU8) -> usize {
    let (w, h) = (m.w, m.h);
    let mut seen = vec![false; m.data.len()];
    let mut n = 0;
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..m.data.len() {
        if m.data[i] == 0 || seen[i] {
            continue;
        }
        n += 1;
        stack.push(i);
        seen[i] = true;
        while let Some(p) = stack.pop() {
            let (x, y) = (p % w, p / w);
            let mut neighbours = Vec::with_capacity(4);
            if x > 0 {
                neighbours.push(p - 1);
            }
            if x + 1 < w {
                neighbours.push(p + 1);
            }
            if y > 0 {
                neighbours.push(p - w);
            }
            if y + 1 < h {
                neighbours.push(p + w);
            }
            for q in neighbours {
                if m.data[q] != 0 && !seen[q] {
                    seen[q] = true;
                    stack.push(q);
                }
            }
        }
    }
    n
}

impl NodeExecutor for Area {
    fn implementation_id(&self) -> &'static str {
        "builtin.area"
    }

    fn execute(&self, ctx: &ExecContext, inputs: &Inputs, _params: &Map<String, Value>) -> Result<Outputs, NodeError> {
        let mask = input(inputs, "mask")?.as_mask()?;
        let (sx, sy) = ctx.pixel_spacing_mm.ok_or_else(|| {
            NodeError::new(
                "missing_pixel_spacing",
                "this image has no pixel spacing, so an area in mm² cannot be computed; add calibration to the image or choose a calibrated one",
            )
        })?;
        let pixels: f64 = mask.data.iter().map(|v| f64::from(*v)).sum();
        let mut rows = BTreeMap::new();
        rows.insert("pixels".to_string(), pixels);
        rows.insert("mm2".to_string(), pixels * sx * sy);
        rows.insert("components".to_string(), components(mask) as f64);
        let mut out = BTreeMap::new();
        out.insert("measurements".to_string(), Artifact::Table(rows));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::reference::*;
    use super::*;
    use std::sync::Arc;

    fn inputs(m: MaskU8) -> Inputs {
        [("mask".to_string(), Arc::new(Artifact::Mask(m)))].into()
    }

    #[test]
    fn matches_the_javascript_counts_and_scales_by_spacing() {
        let r = load();
        let m = mask(&r, &r["area"]["mask"]);
        assert_eq!(components(&m) as u64, r["area"]["components"].as_u64().unwrap());
        let ctx = ExecContext { pixel_spacing_mm: Some((0.05, 0.05)), ..Default::default() };
        let out = Area.execute(&ctx, &inputs(m), &Map::new()).unwrap();
        let Artifact::Table(t) = &out["measurements"] else { panic!("table expected") };
        assert_eq!(t["pixels"] as u64, r["area"]["pixels"].as_u64().unwrap());
        assert!((t["mm2"] - t["pixels"] * 0.0025).abs() < 1e-9);
    }

    #[test]
    fn fails_clearly_without_pixel_spacing() {
        let err = Area.execute(&ExecContext::default(), &inputs(MaskU8 { w: 1, h: 1, data: vec![1] }), &Map::new()).unwrap_err();
        assert_eq!(err.code, "missing_pixel_spacing");
        assert!(err.message.contains("pixel spacing"));
    }

    #[test]
    fn counts_separate_blobs() {
        let m = MaskU8 { w: 5, h: 1, data: vec![1, 0, 1, 1, 0] };
        assert_eq!(components(&m), 2);
    }
}
