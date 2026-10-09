//! Wayland: pen pressure and the eraser end (`tablet-v2`) and touchpad pinch
//! (`pointer-gestures-v1`), neither of which winit exposes.
//!
//! We open a second view onto winit's own connection (same `wl_display`) with
//! a private event queue and read it on a small thread. libwayland allows
//! several threads to read one display, so this does not interfere with
//! winit. Only events for our own `wl_surface` are forwarded.

use std::any::Any;
use std::collections::HashMap;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_pinch_v1::{
    self, ZwpPointerGesturePinchV1,
};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gestures_v1::ZwpPointerGesturesV1;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_manager_v2::ZwpTabletManagerV2;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_group_v2::{
    self, ZwpTabletPadGroupV2,
};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2;
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_v2::{self, ZwpTabletPadV2};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_seat_v2::{self, ZwpTabletSeatV2};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_tool_v2::{self, ZwpTabletToolV2};
use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_v2::ZwpTabletV2;
use winit::window::Window;

use super::{Emit, PlatformEvent};

#[derive(Default)]
struct Tool {
    eraser: bool,
    in_surface: bool,
    down: bool,
    pos: [f32; 2],
    pressure: f32,
    moved: bool,
    went_down: bool,
    went_up: bool,
}

struct State {
    emit: Emit,
    /// Address of our `wl_surface`; events for other surfaces are ignored.
    surface: usize,
    gestures: Option<ZwpPointerGesturesV1>,
    pointer: Option<WlPointer>,
    pinch: Option<ZwpPointerGesturePinchV1>,
    pointer_pos: [f32; 2],
    pointer_in_surface: bool,
    pinch_active: bool,
    pinch_last: f64,
    tools: HashMap<ObjectId, Tool>,
}

fn is_ours(state: &State, id: ObjectId) -> bool {
    id.as_ptr() as usize == state.surface
}

/// Returns a handle that keeps the backend alive, or `None` when the window
/// is not a Wayland window or the compositor offers neither protocol.
pub fn init(window: &Window, emit: Emit) -> Option<Box<dyn Any>> {
    let display = match window.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(h) => h.display.as_ptr(),
        _ => return None,
    };
    let surface = match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Wayland(h) => h.surface.as_ptr() as usize,
        _ => return None,
    };

    // SAFETY: the display pointer comes from winit and outlives this
    // connection: the window (and with it winit's display) is alive for the
    // whole run, and the thread below only ends when the display errors out.
    let backend = unsafe { Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let (globals, mut queue) = match registry_queue_init::<State>(&conn) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("wayland input: cannot read the registry: {e}");
            return None;
        }
    };
    let qh = queue.handle();

    let seat: WlSeat = match globals.bind(&qh, 1..=7, ()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("wayland input: no seat: {e}");
            return None;
        }
    };
    let tablet_manager: Option<ZwpTabletManagerV2> = globals.bind(&qh, 1..=1, ()).ok();
    let gestures: Option<ZwpPointerGesturesV1> = globals.bind(&qh, 1..=1, ()).ok();
    if tablet_manager.is_none() && gestures.is_none() {
        log::info!("wayland: compositor offers neither tablet-v2 nor pointer-gestures");
        return None;
    }
    log::info!(
        "wayland: tablet-v2 {}, pointer-gestures {}",
        if tablet_manager.is_some() {
            "yes"
        } else {
            "no"
        },
        if gestures.is_some() { "yes" } else { "no" }
    );
    if let Some(m) = &tablet_manager {
        let _tablet_seat = m.get_tablet_seat(&seat, &qh, ());
    }

    let mut state = State {
        emit,
        surface,
        gestures,
        pointer: None,
        pinch: None,
        pointer_pos: [0.0; 2],
        pointer_in_surface: false,
        pinch_active: false,
        pinch_last: 1.0,
        tools: HashMap::new(),
    };
    // Receive the seat capabilities and the already attached tablets.
    if let Err(e) = queue.roundtrip(&mut state) {
        log::warn!("wayland input: roundtrip failed: {e}");
        return None;
    }

    let handle = std::thread::Builder::new()
        .name("mizu-wayland-input".into())
        .spawn(move || loop {
            if let Err(e) = queue.blocking_dispatch(&mut state) {
                log::warn!("wayland input stopped: {e}");
                break;
            }
        })
        .ok()?;
    Some(Box::new(handle))
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                let pointer = seat.get_pointer(qh, ());
                if let Some(g) = &state.gestures {
                    state.pinch = Some(g.get_pinch_gesture(&pointer, qh, ()));
                }
                state.pointer = Some(pointer);
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_in_surface = is_ours(state, surface.id());
                state.pointer_pos = [surface_x as f32, surface_y as f32];
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_pos = [surface_x as f32, surface_y as f32];
            }
            wl_pointer::Event::Leave { .. } => state.pointer_in_surface = false,
            _ => {}
        }
    }
}

impl Dispatch<ZwpPointerGesturesV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpPointerGesturesV1,
        _: <ZwpPointerGesturesV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpPointerGesturePinchV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_pointer_gesture_pinch_v1::Event::Begin { surface, .. } => {
                if is_ours(state, surface.id()) {
                    state.pinch_active = true;
                    state.pinch_last = 1.0;
                    (state.emit)(PlatformEvent::PinchBegin {
                        pos: state.pointer_pos,
                    });
                }
            }
            zwp_pointer_gesture_pinch_v1::Event::Update { scale, .. } => {
                if state.pinch_active && state.pinch_last > 0.0 {
                    let delta = (scale / state.pinch_last - 1.0) as f32;
                    state.pinch_last = scale;
                    if delta.abs() > 1e-5 {
                        (state.emit)(PlatformEvent::PinchUpdate {
                            delta,
                            pos: state.pointer_pos,
                        });
                    }
                }
            }
            zwp_pointer_gesture_pinch_v1::Event::End { .. } => {
                if state.pinch_active {
                    state.pinch_active = false;
                    (state.emit)(PlatformEvent::PinchEnd);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpTabletManagerV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletManagerV2,
        _: <ZwpTabletManagerV2 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpTabletSeatV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletSeatV2,
        _: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<ZwpTabletV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletV2,
        _: <ZwpTabletV2 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// Pads (the button strips on tablets) are not used, but their child objects
// must still be created or wayland-client would panic.
impl Dispatch<ZwpTabletPadV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadV2,
        _: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadGroupV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadGroupV2,
        _: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadRingV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadRingV2,
        _: <ZwpTabletPadRingV2 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpTabletPadStripV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadStripV2,
        _: <ZwpTabletPadStripV2 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpTabletToolV2, ()> for State {
    fn event(
        state: &mut Self,
        tool: &ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_tablet_tool_v2::Event as E;
        let our_surface = state.surface;
        let emit = state.emit.clone();
        let t = state.tools.entry(tool.id()).or_default();
        match event {
            E::Type { tool_type } => {
                t.eraser = matches!(tool_type, WEnum::Value(zwp_tablet_tool_v2::Type::Eraser));
            }
            E::ProximityIn { surface, .. } => {
                t.in_surface = surface.id().as_ptr() as usize == our_surface;
            }
            E::ProximityOut => {
                if t.in_surface && t.down {
                    emit(PlatformEvent::PenUp);
                }
                t.in_surface = false;
                t.down = false;
                t.went_down = false;
                t.went_up = false;
                t.moved = false;
            }
            E::Down { .. } => {
                t.down = true;
                t.went_down = true;
            }
            E::Up => {
                t.down = false;
                t.went_up = true;
            }
            E::Motion { x, y } => {
                t.pos = [x as f32, y as f32];
                t.moved = true;
            }
            E::Pressure { pressure } => t.pressure = (pressure as f32 / 65535.0).clamp(0.0, 1.0),
            E::Removed => {
                if t.in_surface && t.down {
                    emit(PlatformEvent::PenUp);
                }
                state.tools.remove(&tool.id());
            }
            E::Frame { .. } => {
                if !t.in_surface {
                    t.went_down = false;
                    t.went_up = false;
                    t.moved = false;
                    return;
                }
                if t.went_down {
                    emit(PlatformEvent::PenDown {
                        pos: t.pos,
                        pressure: t.pressure,
                        eraser: t.eraser,
                    });
                } else if t.moved {
                    if t.down {
                        emit(PlatformEvent::PenMove {
                            pos: t.pos,
                            pressure: t.pressure,
                        });
                    } else {
                        emit(PlatformEvent::PenHover { pos: t.pos });
                    }
                }
                if t.went_up {
                    emit(PlatformEvent::PenUp);
                }
                t.went_down = false;
                t.went_up = false;
                t.moved = false;
            }
            _ => {}
        }
    }
}
