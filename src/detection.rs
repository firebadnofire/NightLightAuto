// SPDX-License-Identifier: MPL-2.0
use crate::config::{Config, DetectionPolicy, OcclusionPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Rect {
    pub fn meaningful(self) -> bool {
        self.right - self.left >= 8 && self.bottom - self.top >= 8
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let r = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (r.right > r.left && r.bottom > r.top).then_some(r)
    }
    pub fn subtract(self, other: Self) -> Vec<Self> {
        let Some(i) = self.intersection(other) else {
            return vec![self];
        };
        [
            Self {
                bottom: i.top,
                ..self
            },
            Self {
                top: i.bottom,
                ..self
            },
            Self {
                top: i.top,
                bottom: i.bottom,
                right: i.left,
                ..self
            },
            Self {
                top: i.top,
                bottom: i.bottom,
                left: i.right,
                ..self
            },
        ]
        .into_iter()
        .filter(|r| r.right > r.left && r.bottom > r.top)
        .collect()
    }
}
#[derive(Debug, Clone, Default)]
pub struct Window {
    pub id: usize,
    pub foreground: bool,
    pub bounds: Rect,
    pub monitor: Rect,
    pub visible: bool,
    pub minimized: bool,
    pub cloaked: bool,
    pub shell: bool,
    pub framed: bool,
    pub maximized: bool,
    pub opaque: bool,
    pub incidental: bool,
    pub path: Option<String>,
    pub name: Option<String>,
}
pub trait WindowSource {
    fn snapshot(&mut self) -> Result<Vec<Window>, String>;
}
pub fn qualifies(window: &Window, config: &Config) -> bool {
    if !window.visible || window.minimized || window.cloaked || window.shell || window.incidental {
        return false;
    }
    if config
        .classify(window.path.as_deref(), window.name.as_deref())
        .is_none()
    {
        return false;
    }
    if window.maximized && config.include_maximized {
        return true;
    }
    if window.framed {
        return false;
    }
    let a = window.bounds;
    let b = window.monitor;
    b.meaningful()
        && [
            (a.left, b.left),
            (a.top, b.top),
            (a.right, b.right),
            (a.bottom, b.bottom),
        ]
        .iter()
        .all(|(x, y)| (i64::from(*x) - i64::from(*y)).abs() <= 2)
}
/// Windows are ordered front to back. Excluded windows may still occlude others.
pub fn detect(windows: &[Window], config: &Config) -> Vec<Window> {
    windows
        .iter()
        .enumerate()
        .filter(|(index, w)| {
            if !qualifies(w, config) {
                return false;
            }
            if config.detection == DetectionPolicy::ForegroundOnly {
                return w.foreground;
            }
            let mut exposed = vec![w.bounds];
            for above in &windows[..*index] {
                if !above.visible
                    || above.minimized
                    || above.cloaked
                    || above.shell
                    || above.incidental
                    || !above.opaque
                {
                    continue;
                }
                if config.occlusion == OcclusionPolicy::EntireDisplayExposed
                    && w.bounds
                        .intersection(above.bounds)
                        .is_some_and(Rect::meaningful)
                {
                    return false;
                }
                exposed = exposed
                    .into_iter()
                    .flat_map(|r| r.subtract(above.bounds))
                    .collect();
                if exposed.is_empty() {
                    break;
                }
            }
            exposed.iter().any(|r| r.meaningful())
        })
        .map(|(_, w)| w.clone())
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn full() -> Window {
        Window {
            bounds: Rect {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1080,
            },
            monitor: Rect {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1080,
            },
            visible: true,
            foreground: true,
            opaque: true,
            ..Window::default()
        }
    }
    #[test]
    fn fullscreen_and_maximized() {
        let mut w = full();
        let mut c = Config::default();
        assert!(qualifies(&w, &c));
        w.framed = true;
        w.maximized = true;
        assert!(!qualifies(&w, &c));
        c.include_maximized = true;
        assert!(qualifies(&w, &c));
    }
    #[test]
    fn visibility_and_exclusion() {
        let mut w = full();
        let mut c = Config::default();
        w.minimized = true;
        assert!(detect(&[w.clone()], &c).is_empty());
        w.minimized = false;
        w.name = Some("firefox.exe".into());
        c.applications[0].excluded = true;
        assert!(detect(&[w], &c).is_empty());
    }
    #[test]
    fn occlusion_and_policy_changes() {
        let mut c = Config {
            detection: DetectionPolicy::AnyVisibleFullscreen,
            ..Config::default()
        };
        let w = full();
        let mut above = Window {
            visible: true,
            opaque: true,
            bounds: Rect {
                left: -1920,
                top: 0,
                right: -960,
                bottom: 1080,
            },
            ..Window::default()
        };
        assert_eq!(detect(&[above.clone(), w.clone()], &c).len(), 1);
        c.occlusion = OcclusionPolicy::EntireDisplayExposed;
        assert!(detect(&[above.clone(), w.clone()], &c).is_empty());
        above.opaque = false;
        assert_eq!(detect(&[above.clone(), w.clone()], &c).len(), 1);
        above.opaque = true;
        above.bounds = w.bounds;
        assert!(detect(&[above, w.clone()], &c).is_empty());
        c.detection = DetectionPolicy::ForegroundOnly;
        assert_eq!(detect(&[w], &c).len(), 1);
    }
    #[test]
    fn second_monitor_does_not_occlude() {
        let c = Config {
            detection: DetectionPolicy::AnyVisibleFullscreen,
            ..Config::default()
        };
        let above = Window {
            visible: true,
            opaque: true,
            bounds: Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            ..Window::default()
        };
        assert_eq!(detect(&[above, full()], &c).len(), 1);
    }
    #[test]
    fn sliver_is_not_exposure() {
        let c = Config {
            detection: DetectionPolicy::AnyVisibleFullscreen,
            ..Config::default()
        };
        let w = full();
        let above = Window {
            visible: true,
            opaque: true,
            bounds: Rect {
                right: -1,
                ..w.bounds
            },
            ..Window::default()
        };
        assert!(detect(&[above, w], &c).is_empty());
    }
}
