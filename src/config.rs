// SPDX-License-Identifier: MPL-2.0
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DetectionPolicy {
    #[default]
    ForegroundOnly,
    AnyVisibleFullscreen,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OcclusionPolicy {
    #[default]
    AnyExposedArea,
    EntireDisplayExposed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RecoveryPolicy {
    #[default]
    NextFullscreenSession,
    ExplicitResumeOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Category {
    #[default]
    Ordinary,
    Browser,
    VideoPlayer,
}
impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ordinary => "Ordinary application",
            Self::Browser => "Browser",
            Self::VideoPlayer => "Video player",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRule {
    pub executable: String,
    pub exact_path: bool,
    pub category: Category,
    pub excluded: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub enabled: bool,
    pub disable_in_fullscreen: bool,
    pub detection: DetectionPolicy,
    pub occlusion: OcclusionPolicy,
    pub recovery: RecoveryPolicy,
    pub include_ordinary: bool,
    pub include_browsers: bool,
    pub include_video: bool,
    pub include_maximized: bool,
    pub entry_ms: u64,
    pub exit_ms: u64,
    pub poll_ms: u64,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub diagnostic_logging: bool,
    pub applications: Vec<AppRule>,
}
impl Default for Config {
    fn default() -> Self {
        let mut applications = Vec::new();
        for (category, names) in [
            (
                Category::Browser,
                "firefox.exe,chrome.exe,msedge.exe,brave.exe,vivaldi.exe,opera.exe,waterfox.exe,librewolf.exe",
            ),
            (
                Category::VideoPlayer,
                "vlc.exe,mpv.exe,mpc-hc.exe,mpc-hc64.exe,mpc-be.exe,mpc-be64.exe,potplayer64.exe",
            ),
        ] {
            applications.extend(names.split(',').map(|name| AppRule {
                executable: name.into(),
                exact_path: false,
                category,
                excluded: false,
            }));
        }
        Self {
            version: 1,
            enabled: true,
            disable_in_fullscreen: true,
            detection: DetectionPolicy::default(),
            occlusion: OcclusionPolicy::default(),
            recovery: RecoveryPolicy::default(),
            include_ordinary: true,
            include_browsers: true,
            include_video: true,
            include_maximized: false,
            entry_ms: 500,
            exit_ms: 750,
            poll_ms: 500,
            start_with_windows: false,
            start_minimized: false,
            close_to_tray: true,
            diagnostic_logging: false,
            applications,
        }
    }
}
pub fn normalize(value: &str) -> String {
    value
        .trim()
        .replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .to_lowercase()
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported configuration version".into());
        }
        if self.entry_ms > 60_000
            || self.exit_ms > 60_000
            || !(100..=60_000).contains(&self.poll_ms)
        {
            return Err(
                "Entry/exit delays must be 0–60000 ms; polling must be 100–60000 ms".into(),
            );
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.applications {
            if r.executable.trim().is_empty() || !r.executable.to_lowercase().ends_with(".exe") {
                return Err("Application identities must end in .exe".into());
            }
            if r.exact_path && !std::path::Path::new(&r.executable).is_absolute() {
                return Err("Exact paths must be absolute".into());
            }
            if !r.exact_path && r.executable.contains(['\\', '/']) {
                return Err("Name rules must contain only an executable filename".into());
            }
            if !seen.insert((r.exact_path, normalize(&r.executable))) {
                return Err(format!("Duplicate application rule: {}", r.executable));
            }
        }
        Ok(())
    }
    pub fn classify(&self, path: Option<&str>, name: Option<&str>) -> Option<Category> {
        let matching: Vec<_> = self
            .applications
            .iter()
            .filter(|r| {
                if r.exact_path {
                    path.is_some_and(|p| normalize(p) == normalize(&r.executable))
                } else {
                    name.is_some_and(|n| normalize(n) == normalize(&r.executable))
                }
            })
            .collect();
        if matching.iter().any(|r| r.excluded) {
            return None;
        }
        let category = matching
            .iter()
            .find(|r| r.exact_path)
            .or_else(|| matching.first())
            .map_or(Category::Ordinary, |r| r.category);
        match category {
            Category::Ordinary if self.include_ordinary => Some(category),
            Category::Browser if self.include_browsers => Some(category),
            Category::VideoPlayer if self.include_video => Some(category),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip() {
        let c = Config::default();
        assert_eq!(
            toml::from_str::<Config>(&toml::to_string(&c).unwrap()).unwrap(),
            c
        );
        assert!(c.validate().is_ok());
    }
    #[test]
    fn validation() {
        let mut c = Config {
            poll_ms: 0,
            ..Config::default()
        };
        assert!(c.validate().is_err());
        c.poll_ms = 500;
        c.applications.push(c.applications[0].clone());
        assert!(c.validate().is_err());
    }
    #[test]
    fn precedence_and_exclusions() {
        let mut c = Config::default();
        c.applications.push(AppRule {
            executable: "C:\\Apps\\firefox.exe".into(),
            exact_path: true,
            category: Category::Ordinary,
            excluded: false,
        });
        assert_eq!(
            c.classify(Some("c:\\apps\\FIREFOX.exe"), Some("firefox.exe")),
            Some(Category::Ordinary)
        );
        c.applications[0].excluded = true;
        assert_eq!(
            c.classify(Some("c:\\apps\\firefox.exe"), Some("firefox.exe")),
            None
        );
    }
    #[test]
    fn unknown_and_runtime_category() {
        let mut c = Config::default();
        assert_eq!(c.classify(None, None), Some(Category::Ordinary));
        c.include_ordinary = false;
        assert_eq!(c.classify(None, None), None);
        c.include_browsers = false;
        assert_eq!(c.classify(None, Some("CHROME.EXE")), None);
    }
}
