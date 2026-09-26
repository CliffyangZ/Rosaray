use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::super::{input, require_f64, Artifact, ExecContext, ImageF32, Inputs, NodeError, NodeExecutor, Outputs};

/// `builtin.gaussian-blur`: two successive box blurs of radius round(0.9·sigma).
pub struct GaussianBlur;

/// One separable box blur with edge clamping, exactly as the prototype does it.
pub fn box_blur(d: &[f32], w: usize, h: usize, r: i64) -> Vec<f32> {
    if r < 1 {
        return d.to_vec();
    }
    let clamp = |v: i64, hi: usize| v.clamp(0, hi as i64 - 1) as usize;
    let mut tmp = vec![0f32; d.len()];
    let mut out = vec![0f32; d.len()];
    let n = (2 * r + 1) as f64;
    for y in 0..h {
        let mut s = 0f64;
        for x in -r..=r {
            s += f64::from(d[y * w + clamp(x, w)]);
        }
        for x in 0..w as i64 {
            tmp[y * w + x as usize] = (s / n) as f32;
            s += f64::from(d[y * w + clamp(x + r + 1, w)]) - f64::from(d[y * w + clamp(x - r, w)]);
        }
    }
    for x in 0..w {
        let mut s = 0f64;
        for y in -r..=r {
            s += f64::from(tmp[clamp(y, h) * w + x]);
        }
        for y in 0..h as i64 {
            out[y as usize * w + x] = (s / n) as f32;
            s += f64::from(tmp[clamp(y + r + 1, h) * w + x]) - f64::from(tmp[clamp(y - r, h) * w + x]);
        }
    }
    out
}

pub fn blur(img: &ImageF32, sigma: f64) -> ImageF32 {
    let r = ((sigma * 0.9).round() as i64).max(1);
    let once = box_blur(&img.data, img.w, img.h, r);
    ImageF32 { w: img.w, h: img.h, data: box_blur(&once, img.w, img.h, r) }
}

impl NodeExecutor for GaussianBlur {
    fn implementation_id(&self) -> &'static str {
        "builtin.gaussian-blur"
    }

    fn execute(&self, _ctx: &ExecContext, inputs: &Inputs, params: &Map<String, Value>) -> Result<Outputs, NodeError> {
        let img = input(inputs, "image")?.as_image()?;
        let sigma = require_f64(params, "sigma")?;
        let mut out = BTreeMap::new();
        out.insert("image".to_string(), Artifact::Image(blur(img, sigma)));
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
        for key in ["gaussian", "gaussian2"] {
            let got = blur(&img, r[key]["sigma"].as_f64().unwrap());
            assert_close(&got.data, &floats(&r[key]["out"]));
        }
    }
}
