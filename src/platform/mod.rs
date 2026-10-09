//! Input that winit does not deliver by itself: touchpad pinch and pen
//! pressure (with the eraser end).
//!
//! - Wayland: `tablet-v2` and `pointer-gestures-v1` (this module's `wayland`).
//! - Windows and macOS: pens already arrive as winit `Touch` events with
//!   `force`, and macOS pinch as `PinchGesture`; `app.rs` handles those.
//!
//! Backends degrade silently: if a protocol or API is missing, mizu keeps
//! using the mouse events winit provides. Set `MIZU_NO_PLATFORM_INPUT=1` to
//! switch every backend off.

use std::any::Any;
use std::sync::Arc;

use winit::window::Window;

#[cfg(all(unix, not(target_os = "macos")))]
mod wayland;

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
    }
    let _ = (window, &emit);
    if backends.is_empty() {
        None
    } else {
        Some(Platform { backends })
    }
}
