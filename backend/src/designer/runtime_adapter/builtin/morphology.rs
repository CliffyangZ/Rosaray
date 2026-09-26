use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::super::{input, require_f64, require_str, Artifact, ExecContext, Inputs, MaskU8, NodeError, NodeExecutor, Outputs};

/// `builtin.morphology`: erode / dilate / open / close with a square element.
pub struct Morphology;

fn pass(src: &[u8], w: usize, h: usize, r: i64, erode: bool) -> Vec<u8> {
    let init = u8::from(erode);
    let fold = |a: u8, v: u8| if erode { a & v } else { a | v };
    let mut tmp = vec![0u8; src.len()];
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let mut a = init;
            for k in -r..=r {
                a = fold(a, src[y * w + (x as i64 + k).clamp(0, w as i64 - 1) as usize]);
            }
            tmp[y * w + x] = a;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut a = init;
            for k in -r..=r {
                a = fold(a, tmp[(y as i64 + k).clamp(0, h as i64 - 1) as usize * w + x]);
            }
            out[y * w + x] = a;
        }
    }
    out
}

pub fn morph(m: &MaskU8, op: &str, r: i64) -> Option<MaskU8> {
    let (w, h) = (m.w, m.h);
    let er = |s: &[u8]| pass(s, w, h, r, true);
    let di = |s: &[u8]| pass(s, w, h, r, false);
    let data = match op {
        "erode" => er(&m.data),
        "dilate" => di(&m.data),
        "open" => di(&er(&m.data)),
        "close" => er(&di(&m.data)),
        _ => return None,
    };
    Some(MaskU8 { w, h, data })
}

impl NodeExecutor for Morphology {
    fn implementation_id(&self) -> &'static str {
        "builtin.morphology"
    }

    fn execute(&self, _ctx: &ExecContext, inputs: &Inputs, params: &Map<String, Value>) -> Result<Outputs, NodeError> {
        let mask = input(inputs, "mask")?.as_mask()?;
        let op = require_str(params, "op")?;
        let r = require_f64(params, "radius")?.round() as i64;
        let out_mask = morph(mask, op, r).ok_or_else(|| NodeError::new("parameter_invalid", format!("unknown operation \"{op}\"")))?;
        let mut out = BTreeMap::new();
        out.insert("mask".to_string(), Artifact::Mask(out_mask));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::reference::*;
    use super::*;

    #[test]
    fn matches_the_javascript_for_every_operation_and_radius() {
        let r = load();
        let m = mask(&r, &r["area"]["mask"]);
        for op in ["open", "close", "erode", "dilate"] {
            for radius in [1, 2] {
                let key = format!("{op}{radius}");
                assert_eq!(morph(&m, op, radius).unwrap(), mask(&r, &r["morph"][&key]), "{key}");
            }
        }
    }

    #[test]
    fn unknown_operations_are_rejected() {
        assert!(morph(&MaskU8 { w: 1, h: 1, data: vec![1] }, "twirl", 1).is_none());
    }
}
