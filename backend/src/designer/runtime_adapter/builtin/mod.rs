//! The trusted built-in executors, ported from the prototype's in-browser
//! algorithms (`frontend/src/algo.js`, `registry.js`). Unit tests compare each
//! against outputs the JavaScript produced on a fixed image
//! (`js_reference.json`), so Preview and Run compute what the prototype did.

pub mod area;
pub mod gaussian;
pub mod morphology;
pub mod normalize;
pub mod source;
pub mod threshold;

use super::NodeExecutor;

pub fn all() -> Vec<Box<dyn NodeExecutor>> {
    vec![
        Box::new(source::ImageSource),
        Box::new(normalize::Normalize),
        Box::new(gaussian::GaussianBlur),
        Box::new(threshold::Threshold),
        Box::new(morphology::Morphology),
        Box::new(area::Area),
    ]
}

#[cfg(test)]
pub mod reference {
    //! Fixed-image outputs produced by the prototype's JavaScript.
    use super::super::{ImageF32, MaskU8};
    use serde_json::Value;

    pub fn load() -> Value {
        serde_json::from_str(include_str!("js_reference.json")).expect("reference json parses")
    }

    fn nums(v: &Value) -> Vec<f64> {
        v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()
    }

    pub fn image(r: &Value) -> ImageF32 {
        ImageF32 { w: r["width"].as_u64().unwrap() as usize, h: r["height"].as_u64().unwrap() as usize, data: nums(&r["image"]).into_iter().map(|v| v as f32).collect() }
    }

    pub fn mask(r: &Value, v: &Value) -> MaskU8 {
        MaskU8 { w: r["width"].as_u64().unwrap() as usize, h: r["height"].as_u64().unwrap() as usize, data: nums(v).into_iter().map(|x| x as u8).collect() }
    }

    pub fn floats(v: &Value) -> Vec<f32> {
        nums(v).into_iter().map(|x| x as f32).collect()
    }

    pub fn assert_close(got: &[f32], want: &[f32]) {
        assert_eq!(got.len(), want.len());
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert!((g - w).abs() < 1e-3, "pixel {i}: got {g}, JavaScript produced {w}");
        }
    }
}
