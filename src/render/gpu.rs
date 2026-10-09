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
    /// The pipeline cache and the file it is kept in (Vulkan only).
    pub pipeline_cache: Option<PipelineCacheFile>,
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

        // Vulkan: keep compiled pipelines between runs (faster start-up).
        let cache_features = adapter.features() & wgpu::Features::PIPELINE_CACHE;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mizu"),
            required_features: cache_features,
            ..Default::default()
        }))
        .context("cannot create the GPU device")?;
        let pipeline_cache = if cache_features.is_empty() {
            None
        } else {
            load_pipeline_cache(&device, &info)
        };
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
            pipeline_cache,
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

pub struct PipelineCacheFile {
    pub cache: wgpu::PipelineCache,
    path: std::path::PathBuf,
    loaded_len: usize,
}

impl PipelineCacheFile {
    /// Write the cache if it grew since it was loaded. On a thread: start-up
    /// must not wait for the disk.
    pub fn save(&self) {
        let Some(data) = self.cache.get_data() else {
            return;
        };
        if data.len() == self.loaded_len {
            return;
        }
        let path = self.path.clone();
        let _ = std::thread::Builder::new()
            .name("mizu-pipeline-cache".into())
            .spawn(move || {
                let write = || -> std::io::Result<()> {
                    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                    std::fs::create_dir_all(dir)?;
                    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
                    std::io::Write::write_all(&mut tmp, &data)?;
                    tmp.persist(&path).map_err(|e| e.error)?;
                    Ok(())
                };
                if let Err(e) = write() {
                    log::debug!("cannot write the pipeline cache: {e}");
                }
            });
    }
}

fn load_pipeline_cache(
    device: &wgpu::Device,
    info: &wgpu::AdapterInfo,
) -> Option<PipelineCacheFile> {
    let key = wgpu::util::pipeline_cache_key(info)?;
    let path = crate::config::project_dirs()?.cache_dir().join(key);
    let data = std::fs::read(&path).ok();
    let loaded_len = data.as_ref().map(|d| d.len()).unwrap_or(0);
    // SAFETY: the data was written by `PipelineCacheFile::save` from
    // `get_data`; with `fallback` wgpu validates its header and starts empty
    // when it does not match this driver.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("mizu"),
            data: data.as_deref(),
            fallback: true,
        })
    };
    log::debug!("pipeline cache {} ({} bytes)", path.display(), loaded_len);
    Some(PipelineCacheFile {
        cache,
        path,
        loaded_len,
    })
}
