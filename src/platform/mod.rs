//! Input that winit does not deliver by itself: touchpad pinch on Wayland /
//! X11 and pen pressure (with the eraser end) on every platform.
//!
//! Each backend feeds the same [`PlatformEvent`]s into the app. Backends
//! degrade silently: if a protocol or API is missing, mizu just keeps using
//! the mouse events winit provides.

use std::sync::Arc;

use winit::window::Window;

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
    PenUp,
}

/// Callback a backend uses to hand events to the app (thread safe).
pub type Emit = Arc<dyn Fn(PlatformEvent) + Send + Sync>;

/// Keeps backend connections alive.
pub struct Platform {
    #[allow(dead_code)]
    backends: Vec<Box<dyn std::any::Any>>,
}

impl Platform {
    /// Called once per frame / event-loop iteration so backends with their
    /// own event queue can dispatch.
    pub fn pump(&mut self) {}
}

pub fn init(_window: &Window, _emit: Emit) -> Option<Platform> {
    None
}
