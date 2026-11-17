use crate::focus::Zone;
use anyhow::{Context, Result, bail, ensure};
use gtk::glib;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SoundSite {
    pub label: String,
    pub url: String,
}
impl SoundSite {
    pub fn new(label: &str, url: &str) -> Result<Self> {
        let site = Self {
            label: label.trim().into(),
            url: url.trim().into(),
        };
        site.validate()?;
        Ok(site)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.label.trim().is_empty()
                && self.label.chars().count() <= 80
                && !self.label.chars().any(char::is_control),
            "Give the website a name of up to 80 characters."
        );
        ensure!(
            self.url.len() <= 2048
                && !self
                    .url
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control()),
            "Enter a valid website URL without spaces."
        );
        let uri = glib::Uri::parse(&self.url, glib::UriFlags::NONE)
            .context("Enter a full website URL, starting with https://")?;
        ensure!(
            matches!(uri.scheme().as_str(), "https" | "http")
                && uri.host().is_some_and(|host| !host.is_empty())
                && uri.userinfo().is_none(),
            "Use an http:// or https:// website URL without embedded credentials."
        );
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub camera: String,
    pub camera_identity: String,
    pub sound: Option<PathBuf>,
    pub sound_sites: Vec<SoundSite>,
    pub delay: u64,
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
            sound_sites: vec![SoundSite {
                label: "Myinstants · Australia".into(),
                url: "https://www.myinstants.com/en/index/au/".into(),
            }],
            delay: 6,
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
            || !self.tolerance.is_finite()
            || !(1.5..=4.).contains(&self.tolerance)
            || self.zones.iter().flatten().any(|z| !z.valid())
        {
            bail!("Invalid saved settings. Move state.json aside and recalibrate.");
        }
        ensure!(
            self.sound_sites.len() <= 16,
            "Keep up to 16 sound-effect website links."
        );
        for site in &self.sound_sites {
            site.validate()?;
        }
        Ok(())
    }
    pub fn add_sound_site(&mut self, site: SoundSite) -> Result<()> {
        site.validate()?;
        ensure!(
            self.sound_sites.len() < 16,
            "Keep up to 16 sound-effect website links."
        );
        ensure!(
            !self.sound_sites.iter().any(|saved| saved.url == site.url),
            "That website link is already saved."
        );
        self.sound_sites.push(site);
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        const MAX_BYTES: u64 = 128 * 1024;
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("Cannot open saved settings"),
        };
        ensure!(
            metadata.is_file() && metadata.len() <= MAX_BYTES,
            "Saved settings must be a regular file smaller than 128 KiB. The original has been preserved."
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Saved settings are too large. The original has been preserved."
        );
        let settings: Self =
            serde_json::from_slice(&bytes).context("Cannot read saved settings")?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().context("Invalid settings path")?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
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
    fn oversized_settings_are_rejected_without_changing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let bytes = vec![b' '; 128 * 1024 + 1];
        std::fs::write(&path, &bytes).unwrap();
        assert!(Settings::load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(Settings::load(dir.path()).is_err());
    }
    #[test]
    fn new_settings_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private/state.json");
        Settings::default().save(&path).unwrap();
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn website_links_are_validated_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut settings = Settings::default();
        let site = SoundSite::new(
            "Another library",
            "https://example.com/sounds?category=funny",
        )
        .unwrap();
        settings.add_sound_site(site.clone()).unwrap();
        assert!(settings.add_sound_site(site.clone()).is_err());
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().sound_sites[1], site);
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https:///",
            "https://user:secret@example.com",
            "not a url",
        ] {
            assert!(SoundSite::new("Test", url).is_err(), "{url}");
        }
        settings.sound_sites.clear();
        settings.save(&path).unwrap();
        assert!(Settings::load(&path).unwrap().sound_sites.is_empty());
    }
    #[test]
    fn old_settings_gain_myinstants_without_losing_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut settings = Settings {
            notes: true,
            ..Default::default()
        };
        settings.zones[2] = Zone::fit(&[[0.; 4]; 30]);
        let mut value = serde_json::to_value(&settings).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("cooldown".into(), serde_json::json!(25));
        value.as_object_mut().unwrap().remove("sound_sites");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert!(loaded.notes);
        assert!(loaded.zones[2].is_some());
        assert_eq!(
            loaded.sound_sites[0].url,
            "https://www.myinstants.com/en/index/au/"
        );
    }
    #[test]
    fn notebook_mode_is_optional_and_retains_saved_calibration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut settings = Settings {
            notes: true,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        assert!(Settings::load(&path).unwrap().notes);
        assert!(Settings::load(&path).unwrap().zones[2].is_none());
        settings.zones[2] = Zone::fit(&[[0.; 4]; 30]);
        settings.notes = false;
        settings.save(&path).unwrap();
        let saved = Settings::load(&path).unwrap();
        assert!(!saved.notes);
        assert!(saved.zones[2].is_some());
        assert_eq!(saved.zones[2].as_ref().unwrap().mean, [0.; 4]);
    }
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
