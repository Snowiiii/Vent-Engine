use vent_common::entities::camera::BasicCameraImpl;
use vent_common::render::DefaultRenderer;
use vent_runtime::render::app_renderer::VentApplicationManager;
use vent_runtime::render::Dimension;
use wgpu::{
    CommandEncoder, Device, Extent3d, Queue, SurfaceConfiguration, SurfaceError, Texture,
    TextureDimension,
};

pub struct EditorRuntimeRenderer {
    texture: Texture,
    app_renderer: VentApplicationManager,
    extent: Extent3d,
}

impl EditorRuntimeRenderer {
    pub fn new(
        default_renderer: &DefaultRenderer,
        dimension: Dimension,
        extent: Extent3d,
        camera: &mut dyn BasicCameraImpl,
    ) -> Self {
        let texture = default_renderer
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Editor Runtime Texture"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: default_renderer.config.format,
                usage: default_renderer.config.usage,
                view_formats: &[],
            });
        let app_renderer = VentApplicationManager::new(dimension, default_renderer, camera);
        Self {
            texture,
            app_renderer,
            extent,
        }
    }

    pub fn render(
        &self,
        _window: &winit::window::Window,
        encoder: &mut CommandEncoder,
        queue: &Queue,
        camera: &mut dyn BasicCameraImpl,
    ) -> Result<(), SurfaceError> {
        let view = self.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Runtime View"),
            ..Default::default()
        });

        self.app_renderer.render(
            encoder,
            &view,
            queue,
            camera,
            self.extent.width as f32 / self.extent.height as f32,
        );
        Ok(())
    }

    pub fn resize(
        &mut self,
        device: &Device,
        queue: &Queue,
        config: &SurfaceConfiguration,
        new_size: &winit::dpi::PhysicalSize<u32>,
        camera: &mut dyn BasicCameraImpl,
    ) {
        self.texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: Extent3d {
                width: new_size.width,
                height: new_size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: config.format,
            usage: config.usage,
            view_formats: &[],
        });
        self.app_renderer.resize(config, device, queue, camera);
    }
}
