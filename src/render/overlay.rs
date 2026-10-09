//! Flat rectangles, discs and rings (status bar, highlights, pen cursor).

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct OverlayInst {
    pub rect: [f32; 4],
    /// Linear RGB, straight alpha.
    pub color: [f32; 4],
    pub params: [f32; 4],
}

impl OverlayInst {
    pub fn rect(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        OverlayInst {
            rect: [x, y, w, h],
            color,
            params: [0.0; 4],
        }
    }

    pub fn disc(x: f32, y: f32, diameter: f32, color: [f32; 4]) -> Self {
        OverlayInst {
            rect: [x, y, diameter, diameter],
            color,
            params: [1.0, 0.0, 0.0, 0.0],
        }
    }

    pub fn ring(cx: f32, cy: f32, radius: f32, thickness: f32, color: [f32; 4]) -> Self {
        OverlayInst {
            rect: [cx - radius, cy - radius, radius * 2.0, radius * 2.0],
            color,
            params: [2.0, thickness, 0.0, 0.0],
        }
    }
}
