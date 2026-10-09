//! Input that winit does not deliver by itself: touchpad pinch and pen
//! pressure (with the eraser end).
//!
//! - Wayland: `tablet-v2` and `pointer-gestures-v1` (this module's `wayland`).
//! - macOS: an `NSEvent` monitor for tablet events (`macos`); pinch comes
//!   from winit as `PinchGesture`. `macos` also has the title bar helpers
//!   and the handler for documents opened from Finder.
//! - X11: XInput 2.4 pinch gestures and the tablet's pressure valuator
//!   (`x11`).
//! - Windows: pens arrive as winit `Touch` events with `force`; `windows`
//!   adds which end of the pen touches (eraser).
//!
//! Backends degrade silently: if a protocol or API is missing, mizu keeps
//! using the mouse events winit provides. Set `MIZU_NO_PLATFORM_INPUT=1` to
//! switch every backend off.

use std::any::Any;
use std::sync::Arc;

use winit::window::Window;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
mod wayland;
#[cfg(windows)]
mod windows;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

/// Positions are in *logical* pixels (what the windowing system reports).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlatformEvent {
    PinchBegin {
        pos: [f32; 2],
    },
    /// Relative scale change since the last update (0.05 = 5 % bigger).
    PinchUpdate {
        delta: f32,
        pos: [f32; 2],
    },
    PinchEnd,
    PenDown {
        pos: [f32; 2],
        pressure: f32,
        eraser: bool,
    },
    PenMove {
        pos: [f32; 2],
        pressure: f32,
    },
    /// The pen is near the surface but not touching it.
    PenHover {
        pos: [f32; 2],
    },
    PenUp,
    /// X11: pressure of the pen that is moving the mouse pointer (the
    /// stroke itself comes through the normal mouse events).
    PenPressure {
        pressure: f32,
        eraser: bool,
    },
}

/// Windows: the pen touching the screen uses its eraser end. Read when a
/// pen `Touch` starts (always false elsewhere).
pub fn pen_eraser_active() -> bool {
    #[cfg(windows)]
    {
        windows::eraser_active()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Callback a backend uses to hand events to the app (thread safe).
pub type Emit = Arc<dyn Fn(PlatformEvent) + Send + Sync>;

/// Keeps backend threads and connections alive.
pub struct Platform {
    #[allow(dead_code)]
    backends: Vec<Box<dyn Any>>,
}

impl Platform {
    /// Called once per event-loop iteration. Current backends run their own
    /// thread, so there is nothing to do; kept for backends that need it.
    pub fn pump(&mut self) {}
}

pub fn init(window: &Window, emit: Emit) -> Option<Platform> {
    if std::env::var_os("MIZU_NO_PLATFORM_INPUT").is_some() {
        return None;
    }
    log::debug!("platform input: starting backends");
    #[allow(unused_mut)]
    let mut backends: Vec<Box<dyn Any>> = Vec::new();
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(b) = wayland::init(window, emit.clone()) {
        backends.push(b);
    } else if let Some(b) = x11::init(window, emit.clone()) {
        backends.push(b);
    }
    #[cfg(windows)]
    if let Some(b) = windows::init(window, emit.clone()) {
        backends.push(b);
    }
    #[cfg(target_os = "macos")]
    if let Some(b) = macos::init(window, emit.clone()) {
        backends.push(b);
    }
    let _ = (window, &emit);
    if backends.is_empty() {
        None
    } else {
        Some(Platform { backends })
    }
}
