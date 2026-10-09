//! GPU storage for rendered tiles and page previews.
//!
//! All slots live in a few big `texture_2d_array`s. Arrays are created lazily
//! (small first, then bigger) up to the memory budget, and slots are recycled
//! so nothing is created or destroyed while scrolling.

use crate::doc::worker::{THUMB, TILE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub array: u16,
    pub layer: u16,
}

struct ArrayTex {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

pub struct SlotArrays {
    slot_px: u32,
    max_slots: usize,
    max_layers_per_array: u32,
    arrays: Vec<ArrayTex>,
    free: Vec<Slot>,
    allocated: usize,
    label: &'static str,
}

impl SlotArrays {
    pub fn new(slot_px: u32, budget_mb: u32, device: &wgpu::Device, label: &'static str) -> Self {
        let bytes = (slot_px * slot_px * 4) as usize;
        let max_slots = ((budget_mb as usize) << 20) / bytes;
        let max_layers = device.limits().max_texture_array_layers.clamp(1, 64);
        SlotArrays {
            slot_px,
            max_slots: max_slots.max(4),
            max_layers_per_array: max_layers,
            arrays: Vec::new(),
            free: Vec::new(),
            allocated: 0,
            label,
        }
    }

    pub fn capacity(&self) -> usize {
        self.max_slots
    }

    pub fn in_use(&self) -> usize {
        self.allocated - self.free.len()
    }

    pub fn array_count(&self) -> usize {
        self.arrays.len()
    }

    pub fn bind_group(&self, array: u16) -> &wgpu::BindGroup {
        &self.arrays[array as usize].bind_group
    }

    /// Take a free slot, growing by one array when the budget allows.
    pub fn alloc(&mut self, device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Option<Slot> {
        if let Some(s) = self.free.pop() {
            return Some(s);
        }
        if self.allocated >= self.max_slots {
            return None;
        }
        // 16, 32, 64, 64 ... layers; never beyond the budget.
        let k = self.arrays.len() as u32;
        let want = (16u32 << k.min(8)).min(self.max_layers_per_array);
        let room = (self.max_slots - self.allocated) as u32;
        let layers = want.min(room).max(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(self.label),
            size: wgpu::Extent3d {
                width: self.slot_px,
                height: self.slot_px,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(self.label),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        let array = self.arrays.len() as u16;
        self.arrays.push(ArrayTex {
            texture,
            bind_group,
        });
        self.allocated += layers as usize;
        // Hand out layer 0 now, keep the rest (in reverse so pop() yields 1, 2, ...).
        for layer in (1..layers).rev() {
            self.free.push(Slot {
                array,
                layer: layer as u16,
            });
        }
        Some(Slot { array, layer: 0 })
    }

    pub fn release(&mut self, slot: Slot) {
        self.free.push(slot);
    }

    /// Copy the valid `w x h` corner of a slot-sized RGBA buffer to the GPU.
    pub fn upload(&self, queue: &wgpu::Queue, slot: Slot, w: u32, h: u32, data: &[u8]) {
        if w == 0 || h == 0 {
            return;
        }
        let stride = self.slot_px * 4;
        let needed = ((h - 1) * stride + w * 4) as usize;
        if data.len() < needed {
            log::error!("upload: buffer too small ({} < {needed})", data.len());
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.arrays[slot.array as usize].texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: slot.layer as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &data[..needed],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(self.slot_px),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
}

pub fn tile_slot_px() -> u32 {
    TILE
}

pub fn thumb_slot_px() -> u32 {
    THUMB
}
