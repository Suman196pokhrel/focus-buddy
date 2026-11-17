use anyhow::{Context, Result, bail};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
};
#[derive(Default)]
pub struct Audio {
    child: Option<Child>,
}
impl Audio {
    pub fn play(&mut self, path: &Path) -> Result<()> {
        self.start(path, false)
    }
    pub fn play_loop(&mut self, path: &Path) -> Result<()> {
        self.start(path, true)
    }
    fn start(&mut self, path: &Path, looping: bool) -> Result<()> {
        if !path.is_file() {
            bail!("Sound file is missing: {}", path.display());
        }
        self.stop();
        self.child = Some(
            player(path, looping)
                .spawn()
                .context("Cannot start audio player. Install ffmpeg.")?,
        );
        Ok(())
    }
    pub fn poll(&mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut()
            && let Some(status) = child.try_wait()?
        {
            self.child = None;
            if !status.success() {
                bail!("Sound could not be played. Check its format and your audio output.");
            }
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.stop();
    }
}

fn player(path: &Path, looping: bool) -> Command {
    let mut command = Command::new("ffplay");
    command
        .args([
            "-nodisp",
            "-autoexit",
            "-loglevel",
            "error",
            "-volume",
            "65",
            "-loop",
            if looping { "0" } else { "1" },
            "-i",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        thread,
        time::{Duration, Instant},
    };
    #[test]
    fn short_sound_repeats_until_stopped_and_preview_plays_once() {
        let mut sound = tempfile::NamedTempFile::new().unwrap();
        sound
            .write_all(include_bytes!("../assets/nudge.wav"))
            .unwrap();
        let spawn = |looping| {
            player(sound.path(), looping)
                .env("SDL_AUDIODRIVER", "dummy")
                .env("SDL_VIDEODRIVER", "dummy")
                .spawn()
                .unwrap()
        };
        let mut audio = Audio {
            child: Some(spawn(true)),
        };
        thread::sleep(Duration::from_millis(1200));
        assert!(
            audio.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "loop ended after the short clip"
        );
        let pid = audio.child.as_ref().unwrap().id();
        audio.stop();
        assert!(audio.child.is_none());
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "audio process was not reaped"
        );
        audio.child = Some(spawn(false));
        let deadline = Instant::now() + Duration::from_secs(5);
        while audio.child.is_some() && Instant::now() < deadline {
            audio.poll().unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            audio.child.is_none(),
            "preview should finish after one play"
        );
    }
}
