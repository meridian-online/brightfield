//! VelloRenderer — wgpu-backed Vello scene renderer.
//!
//! Wraps a dedicated wgpu device/queue and Vello renderer behind an
//! `Arc<Mutex<VelloRenderer>>`. Renders Vello scenes to RGBA pixel buffers
//! for display as GPUI images.
//!
//! Uses `vello::wgpu` (Vello's re-exported wgpu) to avoid version conflicts.
//! The wgpu device is standalone (not shared with GPUI), created once,
//! and shared via `Arc<Mutex<VelloRenderer>>`.

use std::sync::{Arc, Mutex};

use vello::wgpu;
use vello::{Renderer as VelloInner, RendererOptions, Scene};

/// GPU-backed Vello renderer.
///
/// Owns a dedicated wgpu device and queue (not shared with GPUI).
/// Created once and shared via `Arc<Mutex<VelloRenderer>>`.
///
/// Vello's internal `Renderer` is not `Sync` (contains `RefCell`), so
/// we use `Mutex` to provide safe shared access. GPUI's single-threaded
/// paint guarantees no contention in practice.
///
/// # Panics
///
/// `VelloRenderer::new()` panics with a diagnostic message if wgpu device
/// creation fails — the app cannot function without a GPU device.
pub struct VelloRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: VelloInner,
}

impl VelloRenderer {
    /// Create a new VelloRenderer with a dedicated wgpu device.
    ///
    /// Returns `Arc<Mutex<Self>>` for thread-safe sharing.
    ///
    /// # Panics
    ///
    /// Panics if GPU adapter or device creation fails. The diagnostic
    /// message includes the failure reason to aid debugging.
    pub fn new() -> Arc<Mutex<Self>> {
        let instance = wgpu::Instance::default();

        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect(
                    "VelloRenderer: failed to find a suitable GPU adapter. \
             Ensure a Metal-capable GPU is available (macOS) or \
             Vulkan is supported (Linux/Windows).",
                );

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("brightfield-vello"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            ..Default::default()
        }))
        .expect(
            "VelloRenderer: failed to create wgpu device. \
             GPU adapter was found but device creation failed.",
        );

        let renderer = VelloInner::new(
            &device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::all(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("VelloRenderer: failed to create Vello renderer.");

        Arc::new(Mutex::new(Self {
            device,
            queue,
            renderer,
        }))
    }

    /// The dedicated wgpu device this renderer owns (a cheap handle clone).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The dedicated wgpu queue this renderer submits on (a cheap handle clone).
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Build a Vello renderer on an **externally owned** wgpu device/queue —
    /// the egui/eframe path (the shared `egui_wgpu::RenderState` device), so a
    /// Vello scene rasterises straight onto a texture egui samples, with no
    /// second device and no CPU readback. Sharing the host's device is the whole
    /// point: a texture written by one device cannot be sampled by another, so a
    /// renderer-owned device would force every frame back through host memory.
    /// The gpui path, which has no device to borrow, keeps [`Self::new`]'s
    /// dedicated device.
    ///
    /// # Panics
    ///
    /// Panics if the Vello renderer fails to build on the given device.
    pub fn from_shared(device: wgpu::Device, queue: wgpu::Queue) -> Arc<Mutex<Self>> {
        let renderer = VelloInner::new(
            &device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::all(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("VelloRenderer: failed to create Vello renderer on the shared device.");
        Arc::new(Mutex::new(Self {
            device,
            queue,
            renderer,
        }))
    }

    /// Rasterise `scene` straight into `view` (an existing texture view on this
    /// renderer's device) over `base`, with **no readback** — the zero-copy
    /// present primitive the egui host wraps in `register_native_texture`. The
    /// target texture must be `Rgba8Unorm` with `STORAGE_BINDING |
    /// RENDER_ATTACHMENT` usage and be `width`x`height` device pixels.
    ///
    /// # Panics
    ///
    /// Panics if `width`/`height` is zero or Vello's render fails.
    pub fn render_to_texture(
        &mut self,
        scene: &Scene,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        base: vello::peniko::Color,
    ) {
        assert!(
            width > 0 && height > 0,
            "render dimensions must be non-zero"
        );
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                view,
                &vello::RenderParams {
                    base_color: base,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .expect("VelloRenderer: render_to_texture failed");
    }

    /// Read an RGBA texture on this device back into a tightly-packed
    /// `width*height*4` `Vec<u8>` (row padding stripped) — the offscreen capture
    /// path for `brightfield-shot`, the one place the egui pipeline touches a
    /// readback (final PNG only, never per display frame).
    pub fn read_texture(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<u8> {
        self.read_texture_to_pixels(texture, width, height)
    }

    /// Render a Vello scene to an RGBA pixel buffer over a transparent base.
    ///
    /// Returns `Vec<u8>` with length `width * height * 4` containing
    /// RGBA pixel data. The buffer is read back from the GPU texture.
    ///
    /// # Panics
    ///
    /// Panics if width or height is zero.
    pub fn render_to_pixels(&mut self, scene: &Scene, width: u32, height: u32) -> Vec<u8> {
        self.render_to_pixels_with_base(scene, width, height, vello::peniko::Color::TRANSPARENT)
    }

    /// Render a Vello scene to an RGBA pixel buffer over the given base colour.
    ///
    /// The base-colour-agnostic core of [`Self::render_to_pixels`] (which clears
    /// to transparent). Used by the [`crate::canvas_host::CanvasHost`] present
    /// path; the current chart present always clears to transparent (the overlay
    /// and ink carry their own alpha).
    ///
    /// # Panics
    ///
    /// Panics if width or height is zero.
    pub fn render_to_pixels_with_base(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base: vello::peniko::Color,
    ) -> Vec<u8> {
        assert!(
            width > 0 && height > 0,
            "render dimensions must be non-zero"
        );

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vello-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &view,
                &vello::RenderParams {
                    base_color: base,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .expect("VelloRenderer: render_to_texture failed");

        self.read_texture_to_pixels(&texture, width, height)
    }

    fn read_texture_to_pixels(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<u8> {
        let bytes_per_row = width * 4;
        // wgpu requires rows aligned to 256 bytes
        let padded_bytes_per_row = (bytes_per_row + 255) & !255;

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vello-readback"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vello-copy"),
            });

        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Map the buffer and read pixels.
        let buffer_slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("GPU poll failed while waiting for readback");
        receiver
            .recv()
            .expect("GPU readback channel closed")
            .expect("GPU readback failed");

        let data = buffer_slice.get_mapped_range();

        // Remove row padding if present.
        if padded_bytes_per_row == bytes_per_row {
            data.to_vec()
        } else {
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            for row in 0..height {
                let start = (row * padded_bytes_per_row) as usize;
                let end = start + bytes_per_row as usize;
                pixels.extend_from_slice(&data[start..end]);
            }
            pixels
        }
    }
}

#[cfg(all(test, feature = "gpu-tests"))]
mod tests {
    use super::*;
    use kurbo::{Affine, Circle};
    use peniko::{Color, Fill};
    use vello::Scene;

    #[test]
    fn render_to_pixels_correct_buffer_size() {
        let renderer = VelloRenderer::new();
        let mut renderer = renderer.lock().unwrap();
        let mut scene = Scene::new();
        let circle = Circle::new((50.0, 50.0), 20.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([1.0, 0.0, 0.0, 1.0]),
            None,
            &circle,
        );

        let width = 100;
        let height = 80;
        let pixels = renderer.render_to_pixels(&scene, width, height);
        assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "pixel buffer should be width * height * 4 bytes"
        );
    }

    #[test]
    fn render_to_pixels_has_content() {
        let renderer = VelloRenderer::new();
        let mut renderer = renderer.lock().unwrap();
        let mut scene = Scene::new();
        let circle = Circle::new((50.0, 50.0), 20.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([1.0, 0.0, 0.0, 1.0]),
            None,
            &circle,
        );

        let pixels = renderer.render_to_pixels(&scene, 100, 100);
        let non_zero_count = pixels.iter().filter(|&&b| b != 0).count();
        assert!(
            non_zero_count > 0,
            "rendered circle should produce non-zero pixels"
        );
    }

    #[test]
    fn empty_scene_produces_transparent_pixels() {
        let renderer = VelloRenderer::new();
        let mut renderer = renderer.lock().unwrap();
        let scene = Scene::new();
        let pixels = renderer.render_to_pixels(&scene, 64, 64);
        assert_eq!(pixels.len(), 64 * 64 * 4);
        // With TRANSPARENT base color, all pixels should be zero
        let all_zero = pixels.iter().all(|&b| b == 0);
        assert!(
            all_zero,
            "empty scene with transparent base should be all zeros"
        );
    }
}
