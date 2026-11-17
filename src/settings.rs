use crate::focus::Zone;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub camera: String,
    pub camera_identity: String,
    pub sound: Option<PathBuf>,
    pub delay: u64,
    pub cooldown: u64,
    pub tolerance: f64,
    pub notes: bool,
    pub zones: [Option<Zone>; 4],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            camera: "/dev/video0".into(),
            camera_identity: String::new(),
            sound: None,
            delay: 6,
            cooldown: 25,
            tolerance: 2.5,
            notes: false,
            zones: Default::default(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !(2..=30).contains(&self.delay)
            || !(10..=120).contains(&self.cooldown)
            || !self.tolerance.is_finite()
            || !(1.5..=4.).contains(&self.tolerance)
            || self.zones.iter().flatten().any(|z| !z.valid())
        {
            bail!("Invalid saved settings. Move state.json aside and recalibrate.");
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let settings: Self =
                    serde_json::from_slice(&bytes).context("Cannot read saved settings")?;
                settings.validate()?;
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("Cannot open saved settings"),
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().context("Invalid settings path")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(path)
            .context("Cannot save settings atomically")?;
        Ok(())
    }
}
pub fn data_directory() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("focus-buddy")
}
pub fn models_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("FOCUS_BUDDY_MODELS") {
        return path.into();
    }
    let installed = PathBuf::from("/usr/share/focus-buddy/models");
    if std::env::current_exe().is_ok_and(|exe| exe.parent() == Some(Path::new("/usr/bin"))) {
        return installed;
    }
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models");
    if local.join("yunet.onnx").exists() {
        local
    } else {
        PathBuf::from("/usr/share/focus-buddy/models")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_and_validates_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let settings = Settings {
            delay: 8,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().delay, 8);
        std::fs::write(&path, r#"{"delay":0}"#).unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::write(&path, "broken").unwrap();
        assert!(Settings::load(&path).is_err());
        assert_eq!(
            Settings::load(&dir.path().join("new.json")).unwrap().delay,
            6
        );
    }
}
