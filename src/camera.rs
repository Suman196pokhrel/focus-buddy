use crate::{
    settings,
    vision::{Observation, Vision},
};
use anyhow::{Context, Result, bail};
use image::RgbImage;
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Sample {
    pub observation: Observation,
    pub preview: Option<RgbImage>,
    pub captured: Instant,
}
pub struct Worker {
    stop: Arc<AtomicBool>,
    pub preview: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    output: Arc<Mutex<Option<Result<Sample, String>>>>,
    thread: Option<JoinHandle<()>>,
}
pub fn cameras() -> Vec<(String, String)> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/video4linux") {
        for entry in entries.flatten() {
            let node = entry.file_name().to_string_lossy().into_owned();
            let path = format!("/dev/{node}");
            if !node.starts_with("video") || !std::path::Path::new(&path).exists() {
                continue;
            }
            let name =
                fs::read_to_string(entry.path().join("name")).unwrap_or_else(|_| node.clone());
            result.push((path.clone(), format!("{} · {}", name.trim(), path)));
        }
    }
    result.sort();
    result
}
pub fn identity(camera: &str) -> String {
    let Some(node) = std::path::Path::new(camera).file_name() else {
        return String::new();
    };
    let base = PathBuf::from("/sys/class/video4linux").join(node);
    format!(
        "{}:{}",
        fs::canonicalize(base.join("device"))
            .unwrap_or_default()
            .display(),
        fs::read_to_string(base.join("name"))
            .unwrap_or_default()
            .trim()
    )
}
impl Worker {
    pub fn start(device: &str, preview_enabled: bool) -> Result<Self> {
        if !cameras().iter().any(|(path, _)| path == device) {
            bail!("No webcam at {device}. Connect it and refresh the camera list.");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let preview = Arc::new(AtomicBool::new(preview_enabled));
        let child = Arc::new(Mutex::new(None));
        let output = Arc::new(Mutex::new(None));
        let (thread_stop, thread_preview, thread_child, thread_output) =
            (stop.clone(), preview.clone(), child.clone(), output.clone());
        let device = device.to_owned();
        let handle = thread::spawn(move || {
            let result = run(
                &device,
                &thread_stop,
                &thread_preview,
                &thread_child,
                &thread_output,
            );
            if let Some(mut child) = thread_child.lock().unwrap().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if !thread_stop.load(Ordering::Relaxed)
                && let Err(error) = result
            {
                *thread_output.lock().unwrap() = Some(Err(format!("{error:#}")));
            }
        });
        Ok(Self {
            stop,
            preview,
            child,
            output,
            thread: Some(handle),
        })
    }
    pub fn poll(&self) -> Option<Result<Sample, String>> {
        self.output.lock().unwrap().take()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
fn run(
    device: &str,
    stop: &AtomicBool,
    preview: &AtomicBool,
    child: &Mutex<Option<Child>>,
    output: &Mutex<Option<Result<Sample, String>>>,
) -> Result<()> {
    let mut vision = Vision::load(&settings::models_directory())?;
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut process = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-f",
            "v4l2",
            "-video_size",
            "640x480",
            "-i",
            device,
            "-an",
            "-vf",
            "fps=10,scale=640:480",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot start FFmpeg. Install ffmpeg.")?;
    let mut stdout = process.stdout.take().context("Missing camera pipe")?;
    let mut stderr = process.stderr.take().context("Missing camera error pipe")?;
    *child.lock().unwrap() = Some(process);
    let latest = Arc::new(Mutex::new(None));
    let ended = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(Mutex::new(String::new()));
    let (reader_latest, reader_ended, reader_errors) =
        (latest.clone(), ended.clone(), errors.clone());
    let logger = thread::spawn(move || {
        let mut buffer = [0; 1024];
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let mut message = reader_errors.lock().unwrap();
            message.push_str(&String::from_utf8_lossy(&buffer[..n]));
            if message.len() > 4096 {
                let split = message
                    .char_indices()
                    .find(|(i, _)| *i >= message.len() - 4096)
                    .map_or(0, |(i, _)| i);
                *message = message[split..].into();
            }
        }
    });
    let reader = thread::spawn(move || {
        loop {
            let mut bytes = vec![0; 640 * 480 * 3];
            if stdout.read_exact(&mut bytes).is_err() {
                break;
            }
            if let Some(image) = RgbImage::from_raw(640, 480, bytes) {
                *reader_latest.lock().unwrap() = Some((Instant::now(), image));
            }
        }
        reader_ended.store(true, Ordering::Relaxed);
    });
    let mut last_frame = Instant::now();
    let result = (|| {
        while !stop.load(Ordering::Relaxed) {
            let tick = Instant::now();
            let next = latest.lock().unwrap().take();
            if let Some((captured, frame)) = next {
                last_frame = captured;
                if captured.elapsed() < Duration::from_millis(500) {
                    let observation = vision.process(&frame)?;
                    let preview = preview.load(Ordering::Relaxed).then_some(frame);
                    *output.lock().unwrap() = Some(Ok(Sample {
                        observation,
                        preview,
                        captured,
                    }));
                }
            }
            if ended.load(Ordering::Relaxed) {
                bail!(
                    "Camera stopped. {} Try a different video device or close other camera apps.",
                    errors.lock().unwrap().trim()
                );
            }
            if last_frame.elapsed() > Duration::from_secs(5) {
                bail!("No camera frames for five seconds. Check the camera connection.");
            }
            if let Some(wait) = Duration::from_millis(100).checked_sub(tick.elapsed()) {
                thread::sleep(wait);
            }
        }
        Ok(())
    })();
    if let Some(mut process) = child.lock().unwrap().take() {
        let _ = process.kill();
        let _ = process.wait();
    }
    let _ = reader.join();
    let _ = logger.join();
    result
}
