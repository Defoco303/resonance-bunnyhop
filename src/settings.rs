use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{assist::AssistConfig, calibration::CalibrationStore};

const SETTINGS_DIR_NAME: &str = "resonance-bhop";
const SETTINGS_FILE_NAME: &str = "settings.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PersistedSettings {
    #[serde(default)]
    pub assist: AssistConfig,
    #[serde(default)]
    pub preferred_device_path: Option<String>,
    #[serde(default)]
    pub calibrations: CalibrationStore,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            assist: AssistConfig::default(),
            preferred_device_path: None,
            calibrations: CalibrationStore::default(),
        }
    }
}

pub fn load_settings() -> Result<PersistedSettings> {
    let path = settings_file_path()?;
    if !path.exists() {
        return Ok(PersistedSettings::default());
    }

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read settings file at {}", path.display()))?;
    let settings = toml::from_str::<PersistedSettings>(&contents)
        .with_context(|| format!("failed to parse settings file at {}", path.display()))?;
    Ok(settings)
}

pub fn save_settings(settings: &PersistedSettings) -> Result<()> {
    let path = settings_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create settings directory at {}",
                parent.display()
            )
        })?;
    }

    let contents = toml::to_string_pretty(settings).context("failed to serialize settings")?;
    fs::write(&path, contents)
        .with_context(|| format!("failed to write settings file at {}", path.display()))?;
    Ok(())
}

pub fn settings_file_path() -> Result<PathBuf> {
    let base_dir = appdata_dir().or_else(|_| current_dir_fallback())?;
    Ok(base_dir.join(SETTINGS_FILE_NAME))
}

fn appdata_dir() -> Result<PathBuf> {
    let appdata = env::var_os("APPDATA").context("APPDATA is not available")?;
    Ok(Path::new(&appdata).join(SETTINGS_DIR_NAME))
}

fn current_dir_fallback() -> Result<PathBuf> {
    Ok(env::current_dir()?.join(SETTINGS_DIR_NAME))
}
