#![forbid(unsafe_code)]

mod audio;
mod camera;
mod focus;
mod settings;
mod tray;
mod ui;
mod vision;

use anyhow::{Context, Result, bail};
use std::time::Instant;

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Focus Buddy\n\nfocus-buddy              Open the native desktop app\nfocus-buddy --check      Verify models and run blank-frame inference\nfocus-buddy --bench FILE Benchmark a local image (no recording)\nfocus-buddy --camera-check [DEVICE] Test camera for five seconds; never records\nfocus-buddy --smoke      Open briefly and save target/ui-smoke.png\n\nCamera capture begins only after Start session. Space pauses; Ctrl+Q quits."
        );
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "--camera-check") {
        let device = args.get(1).map(String::as_str).unwrap_or("/dev/video0");
        let worker = camera::Worker::start(device, false)?;
        let start = Instant::now();
        let mut frames = 0;
        let mut valid = 0;
        let mut faces = 0;
        let mut times = Vec::new();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if let Some(result) = worker.poll() {
                let sample = result.map_err(anyhow::Error::msg)?;
                frames += 1;
                valid += usize::from(sample.observation.features.is_some());
                faces += usize::from(sample.observation.faces == 1);
                times.push(sample.observation.milliseconds);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop(worker);
        anyhow::ensure!(frames > 0, "No camera frames processed");
        times.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"ok":true,"camera":device,"frames":frames,"valid_eye_readings":valid,"frames_with_one_face":faces,"median_ms":times[times.len()/2],"camera_released":true,"recorded":false})
        );
        return Ok(());
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--check" || arg == "--bench")
    {
        let mut engine = vision::Vision::load(&settings::models_directory())?;
        let frame = if args[0] == "--bench" {
            let path = args.get(1).context("Usage: focus-buddy --bench IMAGE")?;
            image::imageops::resize(
                &image::open(path)?.to_rgb8(),
                640,
                480,
                image::imageops::FilterType::Triangle,
            )
        } else {
            image::RgbImage::new(640, 480)
        };
        let iterations = if args[0] == "--bench" { 30 } else { 1 };
        let mut times = Vec::new();
        let start = Instant::now();
        let mut result = engine.process(&frame)?;
        for _ in 0..iterations {
            result = engine.process(&frame)?;
            times.push(result.milliseconds);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"ok":true,"frames":iterations,"models":settings::models_directory(),"faces":result.faces,"features":result.features,
            "median_ms":times[times.len()/2],"p95_ms":times[(times.len()*95/100).min(times.len()-1)],"elapsed_s":start.elapsed().as_secs_f64(),
            "note":"Synthetic/still-image check; live desk accuracy has not been measured."})
        );
        return Ok(());
    }
    if args.len() > 1 || args.first().is_some_and(|arg| arg != "--smoke") {
        bail!("Unknown option. Use --help.");
    }
    let code = ui::launch(args.first().is_some_and(|arg| arg == "--smoke"));
    if i32::from(code) != 0 {
        bail!("Desktop app exited with {}", i32::from(code));
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Focus Buddy: {error:#}");
        std::process::exit(1);
    }
}
