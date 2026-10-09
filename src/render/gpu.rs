//! Device, queue and swap chain.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    // The window must outlive the surface, so it is declared last.
    pub window: Arc<Window>,
}

pub enum Acquired {
    Frame {
        texture: wgpu::SurfaceTexture,
        /// The surface still works but should be reconfigured *after* this
        /// frame has been presented (reconfiguring now would invalidate it).
        suboptimal: bool,
    },
    /// Nothing to draw into right now. The caller must try again.
    Skip(&'static str),
}

impl Gpu {
    pub fn new(window: Arc<Window>, event_loop: &ActiveEventLoop, capture: bool) -> Result<Gpu> {
        // WGPU_BACKEND=vulkan|gl and friends are honoured, which helps when a
        // driver misbehaves.
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
                event_loop.owned_display_handle(),
            ))
            .with_env(),
        );
        let surface = instance
            .create_surface(window.clone())
            .context("cannot create a surface for the window")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // WGPU_POWER_PREF=low|high overrides. A reader does not need the discrete GPU,
            // but on Linux hybrid laptops the compositor often runs on the discrete one
            // (e.g. Hyprland with AQ_DRM_DEVICES) and shows buffers from the integrated
            // GPU as black, so prefer the GPU the compositor most likely uses there.
            power_preference: wgpu::PowerPreference::from_env().unwrap_or(
                if cfg!(target_os = "linux") {
                    wgpu::PowerPreference::HighPerformance
                } else {
                    wgpu::PowerPreference::LowPower
                },
            ),
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| anyhow!("no suitable GPU adapter found: {e}"))?;
        let info = adapter.get_info();
        log::info!(
            "GPU: {} ({:?}, {:?}), driver {} {}",
            info.name,
            info.device_type,
            info.backend,
            info.driver,
            info.driver_info
        );
        if info.backend == wgpu::Backend::Gl {
            log::warn!(
                "using the OpenGL backend because no usable Vulkan driver was found; \
                 if the window stays blank try WGPU_BACKEND=vulkan and check `vulkaninfo`"
            );
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mizu"),
            ..Default::default()
        }))
        .context("cannot create the GPU device")?;
        device.on_uncaptured_error(Arc::new(|e| log::error!("wgpu: {e}")));

        let caps = surface.get_capabilities(&adapter);
        let format = super::diag::format_override(&caps.formats)
            .or_else(|| caps.formats.iter().copied().find(|f| f.is_srgb()))
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| anyhow!("surface reports no formats"))?;
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: if capture && caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: super::diag::present_mode_override()
                .filter(|m| caps.present_modes.contains(m) || *m == wgpu::PresentMode::AutoVsync)
                .unwrap_or(wgpu::PresentMode::AutoVsync),
            alpha_mode: caps
                .alpha_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
                .unwrap_or(caps.alpha_modes[0]),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        log::info!(
            "surface: {:?} {}x{} present {:?} alpha {:?} (available: {:?} / {:?})",
            config.format,
            config.width,
            config.height,
            config.present_mode,
            config.alpha_mode,
            caps.present_modes,
            caps.alpha_modes
        );
        surface.configure(&device, &config);

        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
            surface,
            config,
            window,
        })
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        let (w, h) = (w.max(1), h.max(1));
        if (w, h) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn acquire(&mut self) -> Acquired {
        use wgpu::CurrentSurfaceTexture as C;
        match self.surface.get_current_texture() {
            C::Success(texture) => Acquired::Frame {
                texture,
                suboptimal: false,
            },
            C::Suboptimal(texture) => Acquired::Frame {
                texture,
                suboptimal: true,
            },
            C::Timeout => Acquired::Skip("timeout"),
            C::Occluded => Acquired::Skip("occluded"),
            C::Outdated => {
                self.surface.configure(&self.device, &self.config);
                Acquired::Skip("outdated")
            }
            C::Lost => {
                if let Ok(s) = self.instance.create_surface(self.window.clone()) {
                    self.surface = s;
                    self.surface.configure(&self.device, &self.config);
                }
                Acquired::Skip("lost")
            }
            C::Validation => Acquired::Skip("validation error"),
        }
    }

    /// Reconfigure with the current settings (after a suboptimal frame).
    pub fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.config);
    }
}
