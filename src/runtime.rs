// SPDX-License-Identifier: MPL-2.0
use crate::{
    config::Config,
    detection::{Window, WindowSource, detect},
    engine::{Clock, Engine, Phase},
    platform::{NativeClock, NativeNightLight, NativeWindows, set_startup},
    storage::{ConfigPersistence, DiskStore, Paths, log},
};
use std::{
    os::windows::io::AsRawHandle,
    sync::{
        Arc, Mutex,
        atomic::{AtomicIsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*, System::Threading::*, UI::WindowsAndMessaging::PostMessageW,
};
pub const STATUS_MESSAGE: u32 = 0x8001;
pub const STOPPED_MESSAGE: u32 = 0x8002;
#[derive(Debug, Clone)]
pub struct Status {
    pub config: Config,
    pub phase: Phase,
    pub nightlight: Option<bool>,
    pub schedule: String,
    pub fullscreen: Vec<Window>,
    pub current: Option<Window>,
    pub owned: bool,
    pub suspended: bool,
    pub warning: Option<String>,
    pub feedback: String,
    pub generation: u64,
    pub config_ack: u64,
}
impl Status {
    fn new(config: Config) -> Self {
        Self {
            config,
            phase: Phase::Desktop,
            nightlight: None,
            schedule: "Unavailable".into(),
            fullscreen: Vec::new(),
            current: None,
            owned: false,
            suspended: false,
            warning: None,
            feedback: String::new(),
            generation: 0,
            config_ack: 0,
        }
    }
}
pub enum Command {
    Configure(Config, u64),
    Resume,
    Restore,
    Refresh,
    Suspend(bool),
    PrepareSessionEnd(mpsc::Sender<Result<(), String>>),
    Exit,
}
struct Event(HANDLE);
// SAFETY: This unnamed event can be signaled/waited from multiple threads and closes after all clones drop.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
#[derive(Clone)]
pub struct Controller {
    tx: mpsc::Sender<Command>,
    wake: Arc<Event>,
    pub status: Arc<Mutex<Status>>,
    pub hwnd: Arc<AtomicIsize>,
}
impl Controller {
    pub fn send(&self, c: Command) -> Result<(), String> {
        self.tx
            .send(c)
            .map_err(|_| "Background engine stopped".to_string())?;
        if unsafe { SetEvent(self.wake.0) } == 0 {
            return Err(format!(
                "Wake background engine: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
pub fn start(
    paths: Paths,
    read_only: bool,
) -> Result<(Controller, thread::JoinHandle<()>), String> {
    let store = DiskStore {
        paths: paths.clone(),
    };
    let mut warning = None;
    let config = if !paths.config.join("config.toml").exists() {
        let c = Config::default();
        store.save_config(&c)?;
        c
    } else {
        match store.load() {
            Ok(c) => c,
            Err(e) => {
                warning = Some(e);
                Config::default()
            }
        }
    };
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
    if event.is_null() {
        return Err(format!(
            "Create worker event: {}",
            std::io::Error::last_os_error()
        ));
    }
    let (tx, rx) = mpsc::channel();
    let mut status = Status::new(config);
    status.warning = warning;
    let controller = Controller {
        tx,
        wake: Arc::new(Event(event)),
        status: Arc::new(Mutex::new(status)),
        hwnd: Arc::new(AtomicIsize::new(0)),
    };
    let worker = controller.clone();
    let handle = thread::Builder::new()
        .name("NightLight engine".into())
        .spawn(move || {
            let observer=worker.clone();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||run(worker,rx,store,read_only))).is_err() {
                let mut status=observer.status.lock().unwrap_or_else(|e|e.into_inner());
                status.phase=Phase::BackendError;status.warning=Some("Background engine panicked. Exit and restart; recovery information is retained.".into());
                drop(status);
                let hwnd=observer.hwnd.load(Ordering::Relaxed)as HWND;
                if !hwnd.is_null(){unsafe{PostMessageW(hwnd,STATUS_MESSAGE,0,0);PostMessageW(hwnd,STOPPED_MESSAGE,0,0);}}
            }
        })
        .map_err(|e| e.to_string())?;
    Ok((controller, handle))
}
fn run(controller: Controller, rx: mpsc::Receiver<Command>, mut store: DiskStore, read_only: bool) {
    let mut config = controller.snapshot().config;
    let clock = NativeClock::default();
    let mut backend = NativeNightLight::default();
    let mut windows = NativeWindows::default();
    let mut engine = Engine::default();
    let mut feedback = String::new();
    let mut persistent_warning = controller.snapshot().warning;
    let mut sleeping = false;
    let mut failure_count = 0u32;
    let mut last_error = None;
    let mut stop = false;
    let mut config_ack = 0;
    let mut next_backend_attempt = 0;
    match store.suspended() {
        Ok(v) => {
            engine.explicit = v;
            engine.suspended = v;
        }
        Err(e) => persistent_warning = Some(e),
    }
    let mut recovery_pending = None;
    if !read_only {
        match store.recovery() {
            Ok(Some(r)) => {
                recovery_pending = Some(r);
            }
            Ok(None) => {}
            Err(e) => persistent_warning = Some(e),
        }
    }
    let mut watchers = None;
    let mut watcher_retry = 0;
    let mut config_modified = std::fs::metadata(store.paths.config.join("config.toml"))
        .and_then(|m| m.modified())
        .ok();
    let mut next_config_check = 0;
    loop {
        for command in rx.try_iter() {
            let quiet = matches!(&command, Command::Refresh | Command::Suspend(_));
            let result = match command {
                Command::Configure(new, revision) => {
                    config_ack = revision;
                    let old = config.clone();
                    new.validate().and_then(|_| {
                        if new.start_with_windows != old.start_with_windows {
                            if read_only {
                                return Err("Startup changes are blocked in read-only mode".into());
                            }
                            set_startup(new.start_with_windows)?;
                        }
                        if let Err(e) = store.save_config(&new) {
                            if new.start_with_windows != old.start_with_windows {
                                set_startup(old.start_with_windows).map_err(|rollback| {
                                    format!("{e}; startup rollback also failed: {rollback}")
                                })?;
                            }
                            return Err(e);
                        }
                        config = new;
                        config_modified = std::fs::metadata(store.paths.config.join("config.toml"))
                            .and_then(|m| m.modified())
                            .ok();
                        Ok(())
                    })
                }
                Command::Resume => engine.resume(&mut store),
                Command::Restore => {
                    if read_only {
                        Err("Night Light writes are blocked in read-only mode".into())
                    } else {
                        engine.restore_now(clock.now(), &mut backend, &mut store)
                    }
                }
                Command::Refresh => Ok(()),
                Command::PrepareSessionEnd(reply) => {
                    sleeping = true;
                    let result = if read_only {
                        Ok(())
                    } else {
                        engine.release(clock.now(), &mut backend, &mut store)
                    };
                    if reply.send(result.clone()).is_err() {
                        persistent_warning =
                            Some("Session-end acknowledgement receiver disconnected".into());
                    }
                    result
                }
                Command::Suspend(value) => {
                    sleeping = value;
                    if value && !read_only {
                        engine.release(clock.now(), &mut backend, &mut store)
                    } else {
                        Ok(())
                    }
                }
                Command::Exit => {
                    stop = true;
                    if read_only {
                        Ok(())
                    } else {
                        engine.release(clock.now(), &mut backend, &mut store)
                    }
                }
            };
            match result {
                Ok(()) => {
                    if !quiet {
                        feedback = "Changes applied".into();
                    }
                }
                Err(e) => {
                    feedback = format!("Operation failed: {e}");
                    persistent_warning = Some(e);
                }
            }
        }
        let now = clock.now();
        if watchers.is_none() && now.monotonic_ms >= watcher_retry {
            match (
                win_nightlight_watch::Watcher::state(),
                win_nightlight_watch::Watcher::settings(),
            ) {
                (Ok(state), Ok(settings)) => {
                    watchers = Some((state, settings));
                }
                (a, b) => {
                    persistent_warning = Some(format!(
                        "Night Light watcher unavailable; retrying: {:?} {:?}",
                        a.err(),
                        b.err()
                    ));
                    watcher_retry = now.monotonic_ms + 30_000;
                }
            }
        }
        if let Some((state, settings)) = &watchers {
            let a = state.read_pending();
            let b = settings.read_pending();
            if a.is_err() || b.is_err() {
                persistent_warning = Some(format!(
                    "Watcher read failed; reconnecting: {:?} {:?}",
                    a.err(),
                    b.err()
                ));
                watchers = None;
                watcher_retry = now.monotonic_ms + 5_000;
            }
        }
        if now.monotonic_ms >= next_config_check {
            next_config_check = now.monotonic_ms + 2000;
            let modified = std::fs::metadata(store.paths.config.join("config.toml"))
                .and_then(|m| m.modified())
                .ok();
            if modified != config_modified {
                config_modified = modified;
                match store.load() {
                    Ok(mut c) => {
                        c.start_with_windows = config.start_with_windows;
                        config = c;
                        feedback = "Configuration reloaded".into();
                    }
                    Err(e) => persistent_warning = Some(e),
                }
            }
        }
        let mut detected = Vec::new();
        let mut current = None;
        windows.enumerate_all =
            config.detection == crate::config::DetectionPolicy::AnyVisibleFullscreen;
        match windows.snapshot() {
            Ok(snapshot) => {
                current = snapshot.iter().find(|w| w.foreground && !w.shell).cloned();
                detected = detect(&snapshot, &config);
            }
            Err(e) => persistent_warning = Some(e),
        }
        let should_attempt = now.monotonic_ms >= next_backend_attempt;
        let result = if !should_attempt {
            Ok(())
        } else if let Some(record) = recovery_pending.as_ref() {
            match engine.recover(record.clone(), now, &mut backend, &mut store) {
                Ok(()) => {
                    recovery_pending = None;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        } else if read_only || sleeping {
            use crate::engine::NightLightBackend;
            engine.phase = Phase::Paused;
            match backend.read() {
                Ok(o) => {
                    engine.observation = Some(o);
                    Ok(())
                }
                Err(e) => {
                    engine.observation = None;
                    Err(e)
                }
            }
        } else if !stop {
            engine.step(&config, !detected.is_empty(), now, &mut backend, &mut store)
        } else {
            Ok(())
        };
        if should_attempt {
            match result {
                Ok(()) => {
                    failure_count = 0;
                    if engine.warning.as_ref() == last_error.as_ref() {
                        engine.warning = None;
                    }
                    last_error = None;
                }
                Err(e) => {
                    engine.phase = Phase::BackendError;
                    failure_count = (failure_count + 1).min(5);
                    next_backend_attempt =
                        now.monotonic_ms + (1000u64 << failure_count).min(30_000);
                    if last_error.as_ref() != Some(&e) {
                        if let Err(log_error) = log(&store.paths, &e) {
                            persistent_warning = Some(log_error);
                        }
                        last_error = Some(e.clone());
                    }
                    engine.warning = Some(e);
                }
            }
        }
        let warning = if read_only {
            Some("Read-only inspection: Night Light and startup writes are blocked.".into())
        } else {
            engine
                .warning
                .clone()
                .or_else(|| persistent_warning.clone())
        };
        {
            let mut s = controller.status.lock().unwrap_or_else(|e| e.into_inner());
            let previous_phase = s.phase;
            s.config = config.clone();
            s.config_ack = config_ack;
            s.phase = engine.phase;
            s.nightlight = engine.observation.as_ref().map(|o| o.active);
            s.schedule = engine
                .observation
                .as_ref()
                .map(|o| {
                    let fmt = |m: Option<u16>| {
                        m.map(|v| format!("{:02}:{:02}", v / 60, v % 60))
                            .unwrap_or("Unavailable".into())
                    };
                    format!(
                        "{}   {} – {}",
                        o.schedule.mode,
                        fmt(o.schedule.start),
                        fmt(o.schedule.end)
                    )
                })
                .unwrap_or("Unavailable".into());
            s.fullscreen = detected;
            if current.is_some() {
                s.current = current;
            }
            s.owned = engine.owned.is_some();
            s.suspended = engine.suspended;
            s.warning = warning;
            s.feedback = feedback.clone();
            s.generation += 1;
            if config.diagnostic_logging
                && previous_phase != s.phase
                && let Err(e) = log(&store.paths, &format!("State: {}", s.phase.label()))
            {
                s.warning = Some(e);
            }
        }
        let hwnd = controller.hwnd.load(Ordering::Relaxed) as HWND;
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(
                    hwnd,
                    if stop {
                        STOPPED_MESSAGE
                    } else {
                        STATUS_MESSAGE
                    },
                    0,
                    0,
                );
            }
        }
        if stop {
            break;
        }
        let pending = matches!(engine.phase, Phase::EntryPending | Phase::ExitPending);
        let wait = if failure_count > 0 {
            (1000u64 << failure_count).min(30_000)
        } else if pending {
            config.poll_ms.min(100)
        } else {
            config.poll_ms
        };
        let mut handles = vec![controller.wake.0];
        if let Some((a, b)) = &watchers {
            handles.push(a.as_raw_handle());
            handles.push(b.as_raw_handle());
        }
        let result = unsafe {
            WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, wait as u32)
        };
        if result == WAIT_FAILED {
            persistent_warning = Some(format!(
                "Worker wait failed: {}",
                std::io::Error::last_os_error()
            ));
            thread::sleep(Duration::from_millis(1000));
        }
    }
}
