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
            "-protocol_whitelist",
            "file",
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
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };
    #[test]
    fn local_playlist_cannot_request_network_media() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let playlist = dir.path().join("sound.m3u8");
        std::fs::write(&playlist, format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1.0,\nhttp://{}/sound.ts\n#EXT-X-ENDLIST\n",
            listener.local_addr().unwrap()
        )).unwrap();
        let mut child = player(&playlist, false)
            .env("SDL_AUDIODRIVER", "dummy")
            .env("SDL_VIDEODRIVER", "dummy")
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let exited = child.try_wait().unwrap().is_some();
        let _ = child.kill();
        child.wait().unwrap();
        let mut error = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .unwrap();
        assert!(exited && error.contains("not on whitelist"), "{error}");
        assert!(
            listener.accept().is_err(),
            "playlist made a network connection"
        );
    }
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
