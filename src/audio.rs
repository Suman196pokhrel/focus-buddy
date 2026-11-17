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
        if !path.is_file() {
            bail!("Sound file is missing: {}", path.display());
        }
        self.stop();
        self.child = Some(
            Command::new("ffplay")
                .args([
                    "-nodisp",
                    "-autoexit",
                    "-loglevel",
                    "error",
                    "-volume",
                    "65",
                    "-i",
                ])
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
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
