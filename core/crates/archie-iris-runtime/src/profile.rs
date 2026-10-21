//! Where a frame's time goes, stage by stage, when `IRIS_PROFILE` is set.
//!
//! Each [`mark`] waits for the device to finish what was queued (so the time is the stage's
//! own, and the frame as a whole gets slower) and adds the time since the previous mark to
//! the stage it names. Unset, a mark is one load of a cached flag: the serving path pays
//! nothing. Marks are per thread; [`take`] returns and clears them.

use candle_core::Device;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("IRIS_PROFILE").is_some_and(|v| !v.is_empty() && v != "0"))
}

#[derive(Default)]
struct State {
    last: Option<Instant>,
    stages: Vec<(&'static str, f64, u32)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn enabled() -> bool {
    on()
}

/// Starts timing from here (after what the device already had queued).
pub fn start(device: &Device) {
    if !on() {
        return;
    }
    let _ = device.synchronize();
    STATE.with(|s| s.borrow_mut().last = Some(Instant::now()));
}

/// Adds the time since the previous mark (or [`start`]) to `stage`.
pub fn mark(device: &Device, stage: &'static str) {
    if !on() {
        return;
    }
    let _ = device.synchronize();
    let now = Instant::now();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(last) = s.last {
            let ms = (now - last).as_secs_f64() * 1000.0;
            match s.stages.iter_mut().find(|(name, _, _)| *name == stage) {
                Some(entry) => {
                    entry.1 += ms;
                    entry.2 += 1;
                }
                None => s.stages.push((stage, ms, 1)),
            }
        }
        s.last = Some(now);
    });
}

/// The stages since the last take, in first-seen order: name, total ms, count.
pub fn take() -> Vec<(&'static str, f64, u32)> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.last = None;
        std::mem::take(&mut s.stages)
    })
}
