// SPDX-License-Identifier: MPL-2.0
use crate::{
    config::Config,
    engine::{RecoveryRecord, RecoveryStore},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub trait ConfigPersistence {
    fn load(&self) -> Result<Config, String>;
    fn save_config(&self, config: &Config) -> Result<(), String>;
}
#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub local: PathBuf,
    pub logs: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self, String> {
        let roaming = std::env::var_os("APPDATA").ok_or("APPDATA is unavailable")?;
        let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
        let local = PathBuf::from(local).join("FullscreenNightLight");
        let paths = Self {
            config: PathBuf::from(roaming).join("FullscreenNightLight"),
            logs: local.join("logs"),
            local,
        };
        for p in [&paths.config, &paths.local, &paths.logs] {
            fs::create_dir_all(p).map_err(|e| format!("Create {}: {e}", p.display()))?;
        }
        Ok(paths)
    }
}
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        f.write_all(data)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        let a: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: Both buffers are null-terminated and valid for this synchronous call.
        if unsafe {
            MoveFileExW(
                a.as_ptr(),
                b.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    })();
    result.map_err(|e: String| format!("Atomic write {} failed: {e}", path.display()))
}
#[derive(Clone)]
pub struct DiskStore {
    pub paths: Paths,
}
impl ConfigPersistence for DiskStore {
    fn load(&self) -> Result<Config, String> {
        let p = self.paths.config.join("config.toml");
        let data = fs::read_to_string(&p).map_err(|e| format!("Read {}: {e}", p.display()))?;
        let c: Config = toml::from_str(&data)
            .map_err(|e| format!("Invalid config; original preserved: {e}"))?;
        c.validate()?;
        Ok(c)
    }
    fn save_config(&self, c: &Config) -> Result<(), String> {
        c.validate()?;
        let path = self.paths.config.join("config.toml");
        if path.exists() {
            let original = fs::read_to_string(&path)
                .map_err(|e| format!("Read existing configuration before save: {e}"))?;
            let valid = toml::from_str::<Config>(&original).is_ok_and(|old| old.validate().is_ok());
            if !valid {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos();
                let backup = self
                    .paths
                    .config
                    .join(format!("config.invalid-{stamp}.toml"));
                atomic_write(&backup, original.as_bytes())?;
            }
        }
        atomic_write(
            &path,
            toml::to_string_pretty(c)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
    }
}
impl DiskStore {
    pub fn recovery(&self) -> Result<Option<RecoveryRecord>, String> {
        let path = self.paths.local.join("recovery.toml");
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        toml::from_str(&data)
            .map(Some)
            .map_err(|e| format!("Invalid recovery record; preserved without applying: {e}"))
    }
    pub fn suspended(&self) -> Result<bool, String> {
        let p = self.paths.local.join("suspended");
        match fs::read_to_string(p) {
            Ok(s) => Ok(s == "explicit"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("Read suspension state: {e}")),
        }
    }
}
impl RecoveryStore for DiskStore {
    fn save(&mut self, r: &RecoveryRecord) -> Result<(), String> {
        atomic_write(
            &self.paths.local.join("recovery.toml"),
            toml::to_string(r).map_err(|e| e.to_string())?.as_bytes(),
        )
    }
    fn clear(&mut self) -> Result<(), String> {
        remove_if_exists(&self.paths.local.join("recovery.toml"))
    }
    fn explicit_suspension(&mut self, v: bool) -> Result<(), String> {
        let p = self.paths.local.join("suspended");
        if v {
            atomic_write(&p, b"explicit")
        } else {
            remove_if_exists(&p)
        }
    }
}
fn remove_if_exists(p: &Path) -> Result<(), String> {
    match fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Remove {}: {e}", p.display())),
    }
}
pub fn log(paths: &Paths, message: &str) -> Result<(), String> {
    let current = paths.logs.join("application.log");
    if fs::metadata(&current).is_ok_and(|m| m.len() > 1024 * 1024) {
        remove_if_exists(&paths.logs.join("application.4.log"))?;
        for i in (1..4).rev() {
            let a = paths.logs.join(format!("application.{i}.log"));
            if a.exists() {
                fs::rename(&a, paths.logs.join(format!("application.{}.log", i + 1)))
                    .map_err(|e| format!("Rotate log: {e}"))?;
            }
        }
        fs::rename(&current, paths.logs.join("application.1.log")).map_err(|e| e.to_string())?;
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&current)
        .map_err(|e| format!("Open log: {e}"))?;
    writeln!(file, "{seconds} {message}").map_err(|e| format!("Write log: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn isolated() -> DiskStore {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "FullscreenNightLight-test-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        DiskStore {
            paths: Paths {
                config: root.clone(),
                local: root.clone(),
                logs: root,
            },
        }
    }
    #[test]
    fn malformed_original_is_preserved_on_graphical_save() {
        let s = isolated();
        fs::write(s.paths.config.join("config.toml"), "not valid toml {{{").unwrap();
        assert!(s.load().is_err());
        s.save_config(&Config::default()).unwrap();
        assert!(s.load().is_ok());
        let backup = fs::read_dir(&s.paths.config)
            .unwrap()
            .filter_map(Result::ok)
            .find(|p| {
                p.file_name()
                    .to_string_lossy()
                    .starts_with("config.invalid-")
            })
            .unwrap();
        assert_eq!(
            fs::read_to_string(backup.path()).unwrap(),
            "not valid toml {{{"
        );
        fs::remove_dir_all(s.paths.config).unwrap();
    }
    #[test]
    fn invalid_save_never_replaces_valid_config() {
        let s = isolated();
        let original = Config::default();
        s.save_config(&original).unwrap();
        let bad = Config {
            poll_ms: 1,
            ..original.clone()
        };
        assert!(s.save_config(&bad).is_err());
        assert_eq!(s.load().unwrap(), original);
        fs::remove_dir_all(s.paths.config).unwrap();
    }
    #[test]
    fn atomic_replacement_and_missing_directory_errors() {
        let s = isolated();
        let p = s.paths.local.join("test.txt");
        atomic_write(&p, b"first").unwrap();
        atomic_write(&p, b"second").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"second");
        assert!(atomic_write(&s.paths.local.join("missing").join("file"), b"x").is_err());
        fs::remove_dir_all(s.paths.local).unwrap();
    }
    #[test]
    fn explicit_suspension_is_persistent_and_idempotent() {
        let mut s = isolated();
        assert!(!s.suspended().unwrap());
        s.explicit_suspension(true).unwrap();
        assert!(s.suspended().unwrap());
        s.explicit_suspension(false).unwrap();
        s.explicit_suspension(false).unwrap();
        assert!(!s.suspended().unwrap());
        s.clear().unwrap();
        s.clear().unwrap();
        fs::remove_dir_all(s.paths.local).unwrap();
    }
    #[test]
    fn logs_are_bounded_by_rotation() {
        let s = isolated();
        fs::write(
            s.paths.logs.join("application.log"),
            vec![b'x'; 1024 * 1024 + 1],
        )
        .unwrap();
        log(&s.paths, "test error").unwrap();
        assert!(s.paths.logs.join("application.1.log").exists());
        assert!(
            fs::metadata(s.paths.logs.join("application.log"))
                .unwrap()
                .len()
                < 100
        );
        fs::remove_dir_all(s.paths.local).unwrap();
    }
}
