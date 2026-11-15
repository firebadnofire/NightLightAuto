// SPDX-License-Identifier: MPL-2.0
use crate::config::{Config, RecoveryPolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Schedule {
    pub mode: String,
    pub start: Option<u16>,
    pub end: Option<u16>,
}
impl Schedule {
    pub fn active(&self, minute: i64) -> Option<bool> {
        if self.mode == "Off" {
            return None;
        }
        let (s, e) = (i64::from(self.start?), i64::from(self.end?));
        if s == e {
            return None;
        }
        let now = minute.rem_euclid(1440);
        Some(if s < e {
            s <= now && now < e
        } else {
            now >= s || now < e
        })
    }
    pub fn boundary_between(&self, before: i64, now: i64) -> bool {
        if now < before || self.active(now).is_none() {
            return false;
        }
        [self.start, self.end].into_iter().flatten().any(|t| {
            (before - i64::from(t)).div_euclid(1440) < (now - i64::from(t)).div_euclid(1440)
        })
    }
}
#[derive(Debug, Clone)]
pub struct Observation {
    pub active: bool,
    pub usable: bool,
    pub scheduled: bool,
    pub fingerprint: Vec<u8>,
    pub schedule: Schedule,
    pub preview: bool,
}
pub trait NightLightBackend {
    fn read(&mut self) -> Result<Observation, String>;
    /// Must recheck expected fingerprint immediately before writing.
    fn set_active(&mut self, active: bool, expected: &[u8]) -> Result<Observation, String>;
}
#[derive(Debug, Clone, Copy)]
pub struct TimeInfo {
    pub monotonic_ms: u64,
    pub wall_minute: i64,
    pub unix_seconds: u64,
}
pub trait Clock {
    fn now(&self) -> TimeInfo;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub version: u32,
    pub created: u64,
    pub wall_minute: i64,
    pub restore: bool,
    pub schedule: Schedule,
    pub owned: Option<Vec<u8>>,
}
pub trait RecoveryStore {
    fn save(&mut self, record: &RecoveryRecord) -> Result<(), String>;
    fn clear(&mut self) -> Result<(), String>;
    fn explicit_suspension(&mut self, value: bool) -> Result<(), String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Desktop,
    EntryPending,
    OverrideActive,
    Suspended,
    ExitPending,
    Paused,
    BackendError,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Desktop => "Monitoring",
            Self::EntryPending => "Confirming fullscreen",
            Self::OverrideActive => "Fullscreen override active",
            Self::Suspended => "Suspended by external/manual change",
            Self::ExitPending => "Confirming fullscreen exit",
            Self::Paused => "Automation paused",
            Self::BackendError => "Night Light integration unavailable",
        }
    }
}
pub struct Engine {
    pub phase: Phase,
    pub session: bool,
    pub suspended: bool,
    pub explicit: bool,
    pub owned: Option<RecoveryRecord>,
    pub observation: Option<Observation>,
    pub warning: Option<String>,
    enter_at: Option<u64>,
    exit_at: Option<u64>,
    last_minute: Option<i64>,
    baseline: Option<RecoveryRecord>,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            phase: Phase::Desktop,
            session: false,
            suspended: false,
            explicit: false,
            owned: None,
            observation: None,
            warning: None,
            enter_at: None,
            exit_at: None,
            last_minute: None,
            baseline: None,
        }
    }
}
impl Engine {
    pub fn recover(
        &mut self,
        record: RecoveryRecord,
        time: TimeInfo,
        backend: &mut impl NightLightBackend,
        store: &mut impl RecoveryStore,
    ) -> Result<(), String> {
        let current = backend.read()?;
        if record.version != 1
            || time.unix_seconds < record.created
            || time.unix_seconds - record.created > 7 * 86400
            || record.owned.as_deref() != Some(current.fingerprint.as_slice())
            || record.schedule != current.schedule
        {
            self.warning=Some("Recovery record cannot prove ownership; no Night Light changes made. Review Night Light in Windows Settings.".into());
            store.clear()?;
            return Ok(());
        }
        self.owned = Some(record);
        self.release(time, backend, store)
    }
    fn suspend(&mut self, config: &Config, store: &mut impl RecoveryStore) -> Result<(), String> {
        self.owned = None;
        self.baseline = None;
        self.suspended = true;
        self.explicit = config.recovery == RecoveryPolicy::ExplicitResumeOnly;
        store.explicit_suspension(self.explicit)?;
        store.clear()?;
        Ok(())
    }
    pub fn release(
        &mut self,
        time: TimeInfo,
        backend: &mut impl NightLightBackend,
        store: &mut impl RecoveryStore,
    ) -> Result<(), String> {
        let record = self.owned.clone().or_else(|| {
            self.baseline
                .clone()
                .filter(|r| r.schedule.boundary_between(r.wall_minute, time.wall_minute))
        });
        if let Some(record) = record {
            let current = backend.read()?;
            if record.owned.as_deref() == Some(current.fingerprint.as_slice())
                && record.schedule == current.schedule
            {
                let desired = if record
                    .schedule
                    .boundary_between(record.wall_minute, time.wall_minute)
                {
                    current
                        .schedule
                        .active(time.wall_minute)
                        .unwrap_or(record.restore)
                } else {
                    record.restore
                };
                if desired != current.active {
                    store.save(&record)?;
                    self.observation = Some(backend.set_active(desired, &current.fingerprint)?);
                }
            } else {
                self.warning=Some("Night Light changed externally; restoration skipped to preserve newer changes.".into());
            }
            store.clear()?;
            self.owned = None;
        }
        self.baseline = None;
        Ok(())
    }
    pub fn restore_now(
        &mut self,
        time: TimeInfo,
        backend: &mut impl NightLightBackend,
        store: &mut impl RecoveryStore,
    ) -> Result<(), String> {
        self.release(time, backend, store)?;
        self.suspended = true;
        self.phase = Phase::Suspended;
        Ok(())
    }
    pub fn resume(&mut self, store: &mut impl RecoveryStore) -> Result<(), String> {
        store.explicit_suspension(false)?;
        self.explicit = false;
        self.suspended = false;
        self.warning = None;
        Ok(())
    }
    pub fn step(
        &mut self,
        config: &Config,
        fullscreen: bool,
        time: TimeInfo,
        backend: &mut impl NightLightBackend,
        store: &mut impl RecoveryStore,
    ) -> Result<(), String> {
        let result = self.step_inner(config, fullscreen, time, backend, store);
        if let Err(e) = &result {
            self.phase = Phase::BackendError;
            self.warning = Some(e.clone());
            self.observation = None;
        }
        result
    }
    fn step_inner(
        &mut self,
        config: &Config,
        fullscreen: bool,
        time: TimeInfo,
        backend: &mut impl NightLightBackend,
        store: &mut impl RecoveryStore,
    ) -> Result<(), String> {
        if self.suspended {
            let explicit = config.recovery == RecoveryPolicy::ExplicitResumeOnly;
            if explicit != self.explicit {
                store.explicit_suspension(explicit)?;
                self.explicit = explicit;
                if !explicit && !self.session {
                    self.suspended = false;
                }
            }
        }
        let current = backend.read()?;
        // Windows recomputes solar times; this is not a deliberate mode change.
        let mut solar_refreshed = false;
        for record in [&mut self.owned, &mut self.baseline].into_iter().flatten() {
            if record.schedule.mode == "SunsetToSunrise"
                && current.schedule.mode == "SunsetToSunrise"
                && record.schedule != current.schedule
            {
                record.schedule = current.schedule.clone();
                solar_refreshed = true;
            }
        }
        if solar_refreshed && let Some(record) = &self.owned {
            store.save(record)?;
        }
        if !current.usable {
            return Err("Windows reports Night Light unavailable on this device. Check Windows Settings and display drivers.".into());
        }
        if current.preview {
            self.warning = Some(
                "Windows Night Light preview is active; automatic management is suspended.".into(),
            );
            self.suspend(config, store)?;
        }
        if self
            .last_minute
            .is_some_and(|previous| time.wall_minute < previous)
            && self.owned.is_some()
        {
            self.warning =
                Some("Clock moved backwards; automatic ownership released conservatively.".into());
            self.suspend(config, store)?;
        }
        self.last_minute = Some(time.wall_minute);
        if let Some(record) = self.owned.clone() {
            if record.schedule != current.schedule {
                self.warning =
                    Some("Night Light schedule changed; suspended for user control.".into());
                self.suspend(config, store)?;
            } else if record.owned.as_deref() != Some(current.fingerprint.as_slice()) {
                if current.scheduled {
                    let mut updated = record;
                    updated.restore = current.active;
                    updated.wall_minute = time.wall_minute;
                    updated.owned = Some(current.fingerprint.clone());
                    store.save(&updated)?;
                    self.owned = Some(updated);
                } else {
                    self.warning = Some(
                        "External Night Light change detected. Automatic management suspended."
                            .into(),
                    );
                    self.suspend(config, store)?;
                }
            }
        } else if self.session
            && !self.suspended
            && let Some(previous) = &self.observation
            && previous.fingerprint != current.fingerprint
            && !current.scheduled
        {
            self.warning = Some("External Night Light change detected.".into());
            self.suspend(config, store)?;
        }
        self.observation = Some(current.clone());
        if !config.enabled || !config.disable_in_fullscreen {
            self.release(time, backend, store)?;
            self.session = false;
            self.enter_at = None;
            self.exit_at = None;
            if !self.explicit {
                self.suspended = false;
            }
            self.phase = Phase::Paused;
            return Ok(());
        }
        if fullscreen {
            self.exit_at = None;
            if !self.session {
                let start = *self.enter_at.get_or_insert(time.monotonic_ms);
                if time.monotonic_ms.saturating_sub(start) < config.entry_ms {
                    self.phase = Phase::EntryPending;
                    return Ok(());
                }
                self.session = true;
                self.enter_at = None;
                self.baseline = Some(RecoveryRecord {
                    version: 1,
                    created: time.unix_seconds,
                    wall_minute: time.wall_minute,
                    restore: current.active,
                    schedule: current.schedule.clone(),
                    owned: Some(current.fingerprint.clone()),
                });
            }
        } else {
            self.enter_at = None;
            if self.session {
                let start = *self.exit_at.get_or_insert(time.monotonic_ms);
                if time.monotonic_ms.saturating_sub(start) < config.exit_ms {
                    self.phase = if self.suspended {
                        Phase::Suspended
                    } else {
                        Phase::ExitPending
                    };
                    return Ok(());
                }
                self.release(time, backend, store)?;
                self.session = false;
                self.exit_at = None;
                if !self.explicit {
                    self.suspended = false;
                }
            }
            self.phase = if self.suspended {
                Phase::Suspended
            } else {
                Phase::Desktop
            };
            return Ok(());
        }
        if self.suspended {
            self.phase = Phase::Suspended;
            return Ok(());
        }
        if current.active {
            let mut record = self.owned.clone().unwrap_or(RecoveryRecord {
                version: 1,
                created: time.unix_seconds,
                wall_minute: time.wall_minute,
                restore: current.active,
                schedule: current.schedule.clone(),
                owned: None,
            });
            record.owned = None;
            store.save(&record)?;
            // Retain an ambiguous record on write/readback failure; never invent ownership.
            let written = backend.set_active(false, &current.fingerprint)?;
            if written.active {
                return Err(
                    "Night Light disable readback did not match the requested state".into(),
                );
            }
            record.owned = Some(written.fingerprint.clone());
            store.save(&record)?;
            self.owned = Some(record);
            self.observation = Some(written);
        } else if let Some(record) = &mut self.owned
            && record
                .schedule
                .boundary_between(record.wall_minute, time.wall_minute)
        {
            record.restore = record
                .schedule
                .active(time.wall_minute)
                .unwrap_or(record.restore);
            record.wall_minute = time.wall_minute;
            store.save(record)?;
        }
        // With entry already off, retain an in-memory fingerprint. At exit only
        // a known schedule boundary can justify changing that unchanged state.
        self.phase = Phase::OverrideActive;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Backend {
        o: Observation,
        writes: usize,
        fail: bool,
    }
    impl NightLightBackend for Backend {
        fn read(&mut self) -> Result<Observation, String> {
            if self.fail {
                Err("backend failed".into())
            } else {
                Ok(self.o.clone())
            }
        }
        fn set_active(&mut self, a: bool, e: &[u8]) -> Result<Observation, String> {
            assert_eq!(e, self.o.fingerprint);
            self.writes += 1;
            self.o.active = a;
            self.o.scheduled = false;
            self.o.fingerprint = vec![self.writes as u8 + 10];
            Ok(self.o.clone())
        }
    }
    #[derive(Default)]
    struct Store {
        record: Option<RecoveryRecord>,
        explicit: bool,
    }
    impl RecoveryStore for Store {
        fn save(&mut self, r: &RecoveryRecord) -> Result<(), String> {
            self.record = Some(r.clone());
            Ok(())
        }
        fn clear(&mut self) -> Result<(), String> {
            self.record = None;
            Ok(())
        }
        fn explicit_suspension(&mut self, v: bool) -> Result<(), String> {
            self.explicit = v;
            Ok(())
        }
    }
    fn backend(active: bool) -> Backend {
        Backend {
            o: Observation {
                active,
                usable: true,
                scheduled: false,
                fingerprint: vec![1],
                schedule: Schedule {
                    mode: "SetHours".into(),
                    start: Some(1200),
                    end: Some(420),
                },
                preview: false,
            },
            writes: 0,
            fail: false,
        }
    }
    fn config() -> Config {
        Config {
            entry_ms: 0,
            exit_ms: 0,
            ..Config::default()
        }
    }
    fn t(ms: u64, m: i64) -> TimeInfo {
        TimeInfo {
            monotonic_ms: ms,
            wall_minute: m,
            unix_seconds: 100000 + ms / 1000,
        }
    }
    #[test]
    fn on_entry_exit_and_idempotence() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        let c = config();
        e.step(&c, true, t(0, 1230), &mut b, &mut s).unwrap();
        assert!(!b.o.active);
        e.step(&c, true, t(1, 1230), &mut b, &mut s).unwrap();
        assert_eq!(b.writes, 1);
        e.step(&c, false, t(2, 1260), &mut b, &mut s).unwrap();
        assert!(b.o.active);
        assert!(s.record.is_none());
    }
    #[test]
    fn already_off_no_write() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(false), Store::default());
        e.step(&config(), true, t(0, 1000), &mut b, &mut s).unwrap();
        e.step(&config(), false, t(1, 1001), &mut b, &mut s)
            .unwrap();
        assert_eq!(b.writes, 0);
    }
    #[test]
    fn sunrise_during_override() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        e.step(&config(), true, t(0, 1800), &mut b, &mut s).unwrap();
        e.step(&config(), false, t(1, 1900), &mut b, &mut s)
            .unwrap();
        assert!(!b.o.active);
        assert_eq!(b.writes, 1);
    }
    #[test]
    fn sunset_observed_and_maintained() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(false), Store::default());
        e.step(&config(), true, t(0, 1100), &mut b, &mut s).unwrap();
        b.o.active = true;
        b.o.scheduled = true;
        b.o.fingerprint = vec![7];
        e.step(&config(), true, t(1, 1200), &mut b, &mut s).unwrap();
        assert!(!b.o.active);
        e.step(&config(), false, t(2, 1260), &mut b, &mut s)
            .unwrap();
        assert!(b.o.active);
    }
    #[test]
    fn midnight_schedule() {
        let sch = backend(false).o.schedule;
        assert_eq!(sch.active(1439), Some(true));
        assert_eq!(sch.active(5), Some(true));
        assert_eq!(sch.active(800), Some(false));
        assert!(sch.boundary_between(1430, 1900));
    }
    #[test]
    fn manual_change_both_policies() {
        for policy in [
            RecoveryPolicy::NextFullscreenSession,
            RecoveryPolicy::ExplicitResumeOnly,
        ] {
            let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
            let c = Config {
                recovery: policy,
                ..config()
            };
            e.step(&c, true, t(0, 1230), &mut b, &mut s).unwrap();
            b.o.active = true;
            b.o.fingerprint = vec![8];
            e.step(&c, true, t(1, 1231), &mut b, &mut s).unwrap();
            assert!(e.suspended);
            assert!(b.o.active);
            e.step(&c, false, t(2, 1232), &mut b, &mut s).unwrap();
            assert_eq!(e.suspended, policy == RecoveryPolicy::ExplicitResumeOnly);
        }
    }
    #[test]
    fn debounce_alt_tab_and_consecutive_apps() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        let c = Config {
            entry_ms: 500,
            exit_ms: 750,
            ..config()
        };
        e.step(&c, true, t(0, 1230), &mut b, &mut s).unwrap();
        assert_eq!(b.writes, 0);
        e.step(&c, true, t(500, 1230), &mut b, &mut s).unwrap();
        e.step(&c, false, t(600, 1230), &mut b, &mut s).unwrap();
        e.step(&c, true, t(900, 1230), &mut b, &mut s).unwrap();
        assert_eq!(b.writes, 1);
        assert!(e.session);
    }
    #[test]
    fn disable_and_shutdown_restore() {
        for exit in [false, true] {
            let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
            e.step(&config(), true, t(0, 1230), &mut b, &mut s).unwrap();
            if exit {
                e.release(t(1, 1230), &mut b, &mut s).unwrap();
            } else {
                let c = Config {
                    enabled: false,
                    ..config()
                };
                e.step(&c, true, t(1, 1230), &mut b, &mut s).unwrap();
            }
            assert!(b.o.active);
        }
    }
    #[test]
    fn error_is_visible() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        b.fail = true;
        assert!(e.step(&config(), true, t(0, 1230), &mut b, &mut s).is_err());
        assert_eq!(e.phase, Phase::BackendError);
    }
    #[test]
    fn recovery_ownership() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        e.step(&config(), true, t(0, 1230), &mut b, &mut s).unwrap();
        let record = s.record.clone().unwrap();
        let mut restarted = Engine::default();
        restarted
            .recover(record, t(2, 1230), &mut b, &mut s)
            .unwrap();
        assert!(b.o.active);
    }
    #[test]
    fn recovery_preserves_newer_changes() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        e.step(&config(), true, t(0, 1230), &mut b, &mut s).unwrap();
        let record = s.record.clone().unwrap();
        b.o.fingerprint = vec![99];
        Engine::default()
            .recover(record, t(2, 1230), &mut b, &mut s)
            .unwrap();
        assert_eq!(b.writes, 1);
    }
    #[test]
    fn sunset_inferred_when_windows_does_not_emit_a_transition() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(false), Store::default());
        e.step(&config(), true, t(0, 1100), &mut b, &mut s).unwrap();
        e.step(&config(), false, t(1, 1260), &mut b, &mut s)
            .unwrap();
        assert!(b.o.active);
        assert_eq!(b.writes, 1);
    }
    #[test]
    fn manual_off_before_boundary_is_preserved() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(false), Store::default());
        e.step(&config(), true, t(0, 1250), &mut b, &mut s).unwrap();
        e.step(&config(), false, t(1, 1260), &mut b, &mut s)
            .unwrap();
        assert_eq!(b.writes, 0);
    }
    #[test]
    fn policy_switch_does_not_resume_current_suspension() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        let mut c = config();
        e.step(&c, true, t(0, 1250), &mut b, &mut s).unwrap();
        b.o.active = true;
        b.o.fingerprint = vec![77];
        e.step(&c, true, t(1, 1250), &mut b, &mut s).unwrap();
        c.recovery = RecoveryPolicy::ExplicitResumeOnly;
        e.step(&c, true, t(2, 1250), &mut b, &mut s).unwrap();
        assert!(e.suspended && s.explicit);
        assert_eq!(b.writes, 1);
        e.resume(&mut s).unwrap();
        e.step(&c, true, t(3, 1250), &mut b, &mut s).unwrap();
        assert!(!b.o.active);
    }
    #[test]
    fn restore_now_does_not_battle_the_user() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        e.step(&config(), true, t(0, 1250), &mut b, &mut s).unwrap();
        e.restore_now(t(1, 1250), &mut b, &mut s).unwrap();
        e.step(&config(), true, t(2, 1250), &mut b, &mut s).unwrap();
        assert!(b.o.active && e.suspended);
        assert_eq!(b.writes, 2);
    }
    #[test]
    fn backwards_clock_yields_without_writing() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        e.step(&config(), true, t(0, 1250), &mut b, &mut s).unwrap();
        e.step(&config(), true, t(1, 1200), &mut b, &mut s).unwrap();
        assert!(e.suspended);
        assert_eq!(b.writes, 1);
    }
    #[test]
    fn schedule_change_yields() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        e.step(&config(), true, t(0, 1250), &mut b, &mut s).unwrap();
        b.o.schedule.mode = "Off".into();
        e.step(&config(), true, t(1, 1250), &mut b, &mut s).unwrap();
        assert!(e.suspended);
        assert_eq!(b.writes, 1);
    }
    #[test]
    fn monitor_disconnect_uses_exit_debounce() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        let c = Config {
            exit_ms: 750,
            ..config()
        };
        e.step(&c, true, t(0, 1250), &mut b, &mut s).unwrap();
        e.step(&c, false, t(100, 1250), &mut b, &mut s).unwrap();
        assert!(!b.o.active);
        e.step(&c, false, t(850, 1250), &mut b, &mut s).unwrap();
        assert!(b.o.active);
    }
    #[test]
    fn unknown_solar_times_do_not_invent_activation() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(false), Store::default());
        b.o.schedule = Schedule {
            mode: "SunsetToSunrise".into(),
            start: Some(0),
            end: Some(0),
        };
        e.step(&config(), true, t(0, 1100), &mut b, &mut s).unwrap();
        e.step(&config(), false, t(1, 1500), &mut b, &mut s)
            .unwrap();
        assert_eq!(b.writes, 0);
    }
    #[test]
    fn solar_refresh_is_not_manual_intervention() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        b.o.schedule.mode = "SunsetToSunrise".into();
        e.step(&config(), true, t(0, 1250), &mut b, &mut s).unwrap();
        b.o.schedule.start = Some(1199);
        e.step(&config(), true, t(1, 1250), &mut b, &mut s).unwrap();
        assert!(!e.suspended);
        assert_eq!(s.record.unwrap().schedule.start, Some(1199));
    }
    #[test]
    fn changing_explicit_recovery_after_session_allows_next_session() {
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        let mut c = Config {
            recovery: RecoveryPolicy::ExplicitResumeOnly,
            ..config()
        };
        e.step(&c, true, t(0, 1250), &mut b, &mut s).unwrap();
        b.o.active = true;
        b.o.fingerprint = vec![77];
        e.step(&c, true, t(1, 1250), &mut b, &mut s).unwrap();
        e.step(&c, false, t(2, 1250), &mut b, &mut s).unwrap();
        assert!(e.suspended);
        c.recovery = RecoveryPolicy::NextFullscreenSession;
        e.step(&c, false, t(3, 1250), &mut b, &mut s).unwrap();
        assert!(!e.suspended);
        e.step(&c, true, t(4, 1250), &mut b, &mut s).unwrap();
        assert!(!b.o.active);
    }
    #[test]
    fn recovery_intent_failure_prevents_night_light_write() {
        struct RejectStore;
        impl RecoveryStore for RejectStore {
            fn save(&mut self, _: &RecoveryRecord) -> Result<(), String> {
                Err("disk full".into())
            }
            fn clear(&mut self) -> Result<(), String> {
                Ok(())
            }
            fn explicit_suspension(&mut self, _: bool) -> Result<(), String> {
                Ok(())
            }
        }
        let (mut e, mut b) = (Engine::default(), backend(true));
        assert!(
            e.step(&config(), true, t(0, 1250), &mut b, &mut RejectStore)
                .is_err()
        );
        assert_eq!(b.writes, 0);
        assert_eq!(e.phase, Phase::BackendError);
    }
    #[test]
    fn runtime_exclusion_releases_override() {
        use crate::{
            config::AppRule,
            detection::{Rect, Window, detect},
        };
        let rect = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let w = Window {
            foreground: true,
            visible: true,
            bounds: rect,
            monitor: rect,
            name: Some("game.exe".into()),
            ..Window::default()
        };
        let (mut e, mut b, mut s) = (Engine::default(), backend(true), Store::default());
        let mut c = config();
        e.step(
            &c,
            !detect(std::slice::from_ref(&w), &c).is_empty(),
            t(0, 1250),
            &mut b,
            &mut s,
        )
        .unwrap();
        c.applications.push(AppRule {
            executable: "game.exe".into(),
            exact_path: false,
            category: crate::config::Category::Ordinary,
            excluded: true,
        });
        e.step(&c, !detect(&[w], &c).is_empty(), t(1, 1250), &mut b, &mut s)
            .unwrap();
        assert!(b.o.active);
        assert!(!e.session);
    }
}
