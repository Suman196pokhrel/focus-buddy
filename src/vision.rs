use crate::focus::Features;
use anyhow::{Context, Result, ensure};
use image::{
    RgbImage,
    imageops::{self, FilterType},
};
use ort::{session::Session, value::Tensor};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path, time::Instant};

pub const MODELS: [(&str, &str); 2] = [
    (
        "yunet.onnx",
        "ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0",
    ),
    (
        "landmarks.onnx",
        "e848578c7ac2474b35e0c4b9a1498ff4145c525552b3d845bdb1f66c8a9d85c2",
    ),
];

pub fn verify_models(directory: &Path) -> Result<()> {
    for (name, expected) in MODELS {
        let mut file = File::open(directory.join(name))
            .with_context(|| format!("Missing {name}. Run bin/fetch-models."))?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        ensure!(
            format!("{:x}", hash.finalize()) == expected,
            "Checksum mismatch for {name}. Run bin/fetch-models."
        );
    }
    Ok(())
}

pub fn init_runtime() -> Result<()> {
    let path =
        std::env::var("ORT_DYLIB_PATH").unwrap_or_else(|_| "/usr/lib/libonnxruntime.so".into());
    ensure!(Path::new(&path).exists(), "ONNX Runtime is missing: {path}");
    ort::init_from(path).with_name("focus-buddy").commit()?;
    Ok(())
}
fn session(path: &Path) -> Result<Session> {
    Ok(Session::builder()?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .with_intra_op_spinning(false)?
        .with_inter_op_spinning(false)?
        .commit_from_file(path)?)
}

pub struct Vision {
    detector: Session,
    landmarks: Session,
}
#[derive(Default)]
pub struct Observation {
    pub features: Option<Features>,
    pub faces: usize,
    pub milliseconds: f64,
}
#[derive(Clone, Copy, Debug)]
struct Face {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    score: f32,
}
impl Face {
    fn overlap(&self, other: &Self) -> f32 {
        let intersection = (self.x + self.w).min(other.x + other.w) - self.x.max(other.x);
        let height = (self.y + self.h).min(other.y + other.h) - self.y.max(other.y);
        let area = intersection.max(0.) * height.max(0.);
        area / (self.w * self.h + other.w * other.h - area).max(1.)
    }
}
fn nms(mut candidates: Vec<Face>) -> Vec<Face> {
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut faces: Vec<Face> = Vec::new();
    for face in candidates {
        if faces.iter().all(|other| face.overlap(other) < 0.3) {
            faces.push(face);
        }
        if faces.len() >= 2 {
            break;
        }
    }
    faces
}

impl Vision {
    pub fn load(directory: &Path) -> Result<Self> {
        verify_models(directory)?;
        init_runtime()?;
        Ok(Self {
            detector: session(&directory.join("yunet.onnx"))?,
            landmarks: session(&directory.join("landmarks.onnx"))?,
        })
    }
    fn detect(&mut self, frame: &RgbImage) -> Result<Vec<Face>> {
        let small = imageops::resize(frame, 320, 240, FilterType::Triangle);
        let mut data = vec![0_f32; 3 * 320 * 256];
        for (i, pixel) in small.pixels().enumerate() {
            let offset = (i / 320) * 320 + i % 320;
            for channel in 0..3 {
                data[channel * 320 * 256 + offset] = f32::from(pixel[2 - channel]);
            }
        }
        let input = Tensor::from_array(([1usize, 3, 256, 320], data))?;
        let output = self.detector.run(ort::inputs!["input" => input])?;
        let mut candidates = Vec::new();
        for stride in [8, 16, 32] {
            let (_, cls) = output[format!("cls_{stride}")].try_extract_tensor::<f32>()?;
            let (_, obj) = output[format!("obj_{stride}")].try_extract_tensor::<f32>()?;
            let (_, bbox) = output[format!("bbox_{stride}")].try_extract_tensor::<f32>()?;
            let cols = 320 / stride;
            let count = cols * (256 / stride);
            ensure!(
                cls.len() == count && obj.len() == count && bbox.len() == count * 4,
                "Unexpected face detector output"
            );
            for i in 0..count {
                let score = (cls[i].clamp(0., 1.) * obj[i].clamp(0., 1.)).sqrt();
                if !score.is_finite() || score < 0.8 {
                    continue;
                }
                let cx = ((i % cols) as f32 + bbox[i * 4]) * stride as f32 * 2.;
                let cy = ((i / cols) as f32 + bbox[i * 4 + 1]) * stride as f32 * 2.;
                let w = bbox[i * 4 + 2].exp() * stride as f32 * 2.;
                let h = bbox[i * 4 + 3].exp() * stride as f32 * 2.;
                if [cx, cy, w, h].iter().all(|v| v.is_finite())
                    && w > 0.
                    && h > 0.
                    && w <= 1280.
                    && h <= 960.
                {
                    candidates.push(Face {
                        x: cx - w / 2.,
                        y: cy - h / 2.,
                        w,
                        h,
                        score,
                    });
                }
            }
        }
        Ok(nms(candidates))
    }
    pub fn process(&mut self, frame: &RgbImage) -> Result<Observation> {
        ensure!(
            frame.dimensions() == (640, 480),
            "Camera frame must be 640×480"
        );
        let start = Instant::now();
        let faces = self.detect(frame)?;
        let mut result = Observation {
            faces: faces.len(),
            ..Default::default()
        };
        if faces.len() == 1 {
            let face = faces[0];
            let side = face.w.max(face.h).round() as u32;
            let x = (face.x + face.w / 2. - side as f32 / 2.).round() as i32;
            let y = (face.y + face.h * 0.65 - side as f32 / 2.).round() as i32;
            if side >= 80
                && x >= 0
                && y >= 0
                && (x as u32 + side) <= frame.width()
                && (y as u32 + side) <= frame.height()
            {
                let patch = imageops::crop_imm(frame, x as u32, y as u32, side, side).to_image();
                let input = imageops::resize(&patch, 128, 128, FilterType::Triangle);
                let tensor = Tensor::from_array((
                    [1usize, 128, 128, 3],
                    input
                        .into_raw()
                        .into_iter()
                        .map(f32::from)
                        .collect::<Vec<_>>(),
                ))?;
                let output = self.landmarks.run(ort::inputs!["image_input" => tensor])?;
                let (_, values) = output["dense_1"].try_extract_tensor::<f32>()?;
                ensure!(
                    values.len() == 136
                        && values
                            .iter()
                            .all(|x| x.is_finite() && (-0.2..=1.2).contains(x)),
                    "Invalid facial landmarks"
                );
                let mut marks = [[0_f64; 2]; 68];
                for (i, mark) in marks.iter_mut().enumerate() {
                    *mark = [
                        x as f64 + values[2 * i] as f64 * side as f64,
                        y as f64 + values[2 * i + 1] as f64 * side as f64,
                    ];
                }
                result.features = features(frame, &marks);
            }
        }
        result.milliseconds = start.elapsed().as_secs_f64() * 1000.;
        Ok(result)
    }
}
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
fn inside(x: f64, y: f64, polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[j];
        if ((a[1] > y) != (b[1] > y)) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}
fn pupil(frame: &RgbImage, points: &[[f64; 2]]) -> Option<[f64; 2]> {
    let width = distance(points[0], points[3]);
    let height = (distance(points[1], points[5]) + distance(points[2], points[4])) / 2.;
    if width < 14. || height / width < 0.16 {
        return None;
    }
    let min_x = points
        .iter()
        .map(|p| p[0])
        .fold(f64::INFINITY, f64::min)
        .ceil()
        .max(0.) as u32;
    let max_x = points
        .iter()
        .map(|p| p[0])
        .fold(0., f64::max)
        .floor()
        .min(frame.width() as f64 - 1.) as u32;
    let min_y = points
        .iter()
        .map(|p| p[1])
        .fold(f64::INFINITY, f64::min)
        .ceil()
        .max(0.) as u32;
    let max_y = points
        .iter()
        .map(|p| p[1])
        .fold(0., f64::max)
        .floor()
        .min(frame.height() as f64 - 1.) as u32;
    let mut pixels = Vec::new();
    for y in min_y..max_y {
        for x in min_x..max_x {
            if inside(x as f64, y as f64, points)
                && inside(x as f64, y as f64 - 1., points)
                && inside(x as f64, y as f64 + 1., points)
            {
                let p = frame.get_pixel(x, y);
                let gray =
                    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]);
                pixels.push((x as f64, y as f64, gray));
            }
        }
    }
    if pixels.len() < 15 {
        return None;
    }
    let low = pixels.iter().map(|p| p.2).fold(f64::INFINITY, f64::min);
    let high = pixels.iter().map(|p| p.2).fold(0., f64::max);
    if high - low < 18. {
        return None;
    }
    let mut sum = [0.; 3];
    for (x, y, gray) in pixels {
        let weight = (low + (high - low) * 0.4 - gray).max(0.);
        sum[0] += x * weight;
        sum[1] += y * weight;
        sum[2] += weight;
    }
    if sum[2] < 1. {
        return None;
    }
    let delta = [
        sum[0] / sum[2] - (points[0][0] + points[3][0]) / 2.,
        sum[1] / sum[2] - (points[0][1] + points[3][1]) / 2.,
    ];
    let axis = [
        (points[3][0] - points[0][0]) / width,
        (points[3][1] - points[0][1]) / width,
    ];
    // shortcut: dark-pixel eye position is approximate; upgrade to a learned gaze model if target calibration overlaps.
    Some([
        (delta[0] * axis[0] + delta[1] * axis[1]) / width,
        (-delta[0] * axis[1] + delta[1] * axis[0]) / width,
    ])
}
fn combine_eyes(a: Option<[f64; 2]>, b: Option<[f64; 2]>) -> Option<[f64; 2]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]),
        (Some(eye), None) | (None, Some(eye)) => Some(eye),
        (None, None) => None,
    }
}
fn features(frame: &RgbImage, marks: &[[f64; 2]; 68]) -> Option<Features> {
    let eye = combine_eyes(pupil(frame, &marks[36..42]), pupil(frame, &marks[42..48]))?;
    let eye_a = [
        (marks[36][0] + marks[39][0]) / 2.,
        (marks[36][1] + marks[39][1]) / 2.,
    ];
    let eye_b = [
        (marks[42][0] + marks[45][0]) / 2.,
        (marks[42][1] + marks[45][1]) / 2.,
    ];
    let width = distance(eye_a, eye_b);
    if width < 25. {
        return None;
    }
    let axis = [(eye_b[0] - eye_a[0]) / width, (eye_b[1] - eye_a[1]) / width];
    let delta = [
        marks[30][0] - (eye_a[0] + eye_b[0]) / 2.,
        marks[30][1] - (eye_a[1] + eye_b[1]) / 2.,
    ];
    let values = [
        (delta[0] * axis[0] + delta[1] * axis[1]) / width,
        (-delta[0] * axis[1] + delta[1] * axis[0]) / width,
        eye[0],
        eye[1],
    ];
    values.iter().all(|v| v.is_finite()).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_a_visible_eye_without_inventing_missing_readings() {
        assert_eq!(combine_eyes(Some([0.1, 0.2]), None), Some([0.1, 0.2]));
        assert_eq!(combine_eyes(None, Some([0.1, 0.2])), Some([0.1, 0.2]));
        assert_eq!(combine_eyes(None, None), None);
        assert_eq!(
            combine_eyes(Some([0.1, 0.2]), Some([0.3, 0.4])),
            Some([0.2, 0.30000000000000004])
        );
    }
    #[test]
    fn suppresses_duplicate_faces() {
        let a = Face {
            x: 10.,
            y: 10.,
            w: 100.,
            h: 100.,
            score: 0.95,
        };
        assert_eq!(nms(vec![a, a]).len(), 1);
        assert_eq!(nms(vec![a, Face { x: 300., ..a }]).len(), 2);
    }
    #[test]
    fn dark_eye_centroid_moves_with_pupil_and_rejects_closed_eyes() {
        let points = [
            [10., 20.],
            [17., 14.],
            [33., 14.],
            [40., 20.],
            [33., 26.],
            [17., 26.],
        ];
        let mut frame = RgbImage::from_pixel(60, 40, image::Rgb([220; 3]));
        for y in 16..24 {
            for x in 15..21 {
                frame.put_pixel(x, y, image::Rgb([25; 3]));
            }
        }
        assert!(pupil(&frame, &points).unwrap()[0] < -0.1);
        let closed = [
            [10., 20.],
            [17., 19.],
            [33., 19.],
            [40., 20.],
            [33., 21.],
            [17., 21.],
        ];
        assert!(pupil(&frame, &closed).is_none());
        assert!(pupil(&RgbImage::new(60, 40), &points).is_none());
    }
}
