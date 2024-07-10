use ash::vk::{self, Extent2D};

use crate::{
    allocator::MemoryAllocator, begin_single_time_command, buffer::VulkanBuffer,
    end_single_time_command, instance::VulkanInstance, SamplerInfo,
};

pub struct DepthImage {
    pub image: vk::Image,
    pub image_view: vk::ImageView,
    pub memory: vk::DeviceMemory,
}

impl DepthImage {
    pub fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            device.destroy_image_view(self.image_view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

pub struct VulkanImage {
    pub image: vk::Image,
    pub image_view: vk::ImageView,
    pub sampler: vk::Sampler,
    pub memory: vk::DeviceMemory,
}

impl VulkanImage {
    pub fn new(
        instance: &mut VulkanInstance,
        data: &[u8],
        image_size: Extent2D,
        format: vk::Format,
        sampler_info: Option<SamplerInfo>,
    ) -> Self {
        let image_data_size = (image_size.width * image_size.height * 4) as vk::DeviceSize;

        let mut staging_buffer = VulkanBuffer::new_init(
            instance,
            image_data_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            data,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            None,
        );

        let image = Self::create_image(
            &instance.device,
            format,
            image_size,
            1,
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
        );
        let memory = VulkanBuffer::new_image(&instance.device, &instance.memory_allocator, image);
        Self::copy_buffer_to_image(
            instance,
            image,
            &staging_buffer,
            instance.global_command_pool,
            image_size,
            1,
            true,
        );
        staging_buffer.destroy(&instance.device);
        let image_view = Self::create_image_view(
            image,
            &instance.device,
            format,
            1,
            vk::ImageAspectFlags::COLOR,
        );

        let sampler_info = sampler_info.unwrap_or_default();
        let sampler = instance
            .vulkan_cache
            .get_sampler(&instance.device, sampler_info);

        Self {
            image,
            image_view,
            sampler,
            memory,
        }
    }

    pub fn from_image(
        instance: &mut VulkanInstance,
        image: image::DynamicImage,
        sampler_info: Option<SamplerInfo>,
    ) -> Self {
        let image_size = Extent2D {
            width: image.width(),
            height: image.height(),
        };
        let image_data = match &image {
            image::DynamicImage::ImageLuma8(_) | image::DynamicImage::ImageRgb8(_) => {
                image.into_rgba8().into_raw()
            }
            image::DynamicImage::ImageLumaA8(_) | image::DynamicImage::ImageRgba8(_) => {
                image.into_bytes()
            }
            _ => image.into_rgb8().into_raw(),
        };
        let image_data_size = (image_size.width * image_size.height * 4) as vk::DeviceSize;

        let mut staging_buffer = VulkanBuffer::new_init(
            instance,
            image_data_size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            &image_data,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            None,
        );

        let mip_level = (image_size.width.max(image_size.height) as f32)
            .log2()
            .floor() as u32
            + 1;

        let format = vk::Format::R8G8B8A8_UNORM;

        let image = Self::create_image(
            &instance.device,
            format,
            image_size,
            mip_level,
            vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::SAMPLED,
        );
        let memory = VulkanBuffer::new_image(&instance.device, &instance.memory_allocator, image);

        Self::copy_buffer_to_image(
            instance,
            image,
            &staging_buffer,
            instance.global_command_pool,
            image_size,
            mip_level,
            false,
        );
        staging_buffer.destroy(&instance.device);

        Self::generate_mipmaps(
            instance,
            image,
            instance.global_command_pool,
            image_size.width,
            image_size.height,
            mip_level,
        );

        let image_view = Self::create_image_view(
            image,
            &instance.device,
            format,
            mip_level,
            vk::ImageAspectFlags::COLOR,
        );

        let sampler_info = sampler_info.unwrap_or_default();
        let sampler = instance
            .vulkan_cache
            .get_sampler(&instance.device, sampler_info);

        Self {
            image,
            image_view,
            sampler,
            memory,
        }
    }

    pub fn new_depth(
        device: &ash::Device,
        allocator: &MemoryAllocator,
        format: vk::Format,
        size: Extent2D,
    ) -> DepthImage {
        let image = Self::create_image(
            device,
            format,
            size,
            1,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
        );
        let memory = VulkanBuffer::new_image(device, allocator, image);
        let image_view =
            Self::create_image_view(image, device, format, 1, vk::ImageAspectFlags::DEPTH);

        DepthImage {
            image,
            image_view,
            memory,
        }
    }

    pub fn from_color(instance: &mut VulkanInstance, color: [u8; 4], size: Extent2D) -> Self {
        let color_img = image::RgbaImage::from_pixel(size.width, size.height, image::Rgba(color));
        Self::from_image(instance, image::DynamicImage::ImageRgba8(color_img), None)
    }

    pub fn copy_buffer_to_image(
        instance: &VulkanInstance,
        image: vk::Image,
        staging_buffer: &VulkanBuffer,
        command_pool: vk::CommandPool,
        size: Extent2D,
        mip_level: u32,
        make_ready: bool,
    ) {
        let device = &instance.device;

        let command_buffer = begin_single_time_command(device, command_pool);

        let image_barrier = vk::ImageMemoryBarrier2::default()
            .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .image(image)
            .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
            .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                level_count: mip_level,
                layer_count: 1,
                ..Default::default()
            });

        let binding = [image_barrier];
        let dep_info = vk::DependencyInfo::default()
            .image_memory_barriers(&binding)
            .dependency_flags(vk::DependencyFlags::BY_REGION);

        unsafe { device.cmd_pipeline_barrier2(command_buffer, &dep_info) };

        let subresource = vk::ImageSubresourceLayers::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .mip_level(0)
            .base_array_layer(0)
            .layer_count(1);

        let region = vk::BufferImageCopy2::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(subresource)
            .image_offset(vk::Offset3D::default())
            .image_extent(size.into());

        let binding = [region];
        let copy_image_info = vk::CopyBufferToImageInfo2::default()
            .src_buffer(staging_buffer.buffer)
            .dst_image(image)
            .dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .regions(&binding);

        unsafe { device.cmd_copy_buffer_to_image2(command_buffer, &copy_image_info) };

        if make_ready {
            let image_barrier = vk::ImageMemoryBarrier2::default()
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags2::SHADER_READ)
                .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    level_count: mip_level,
                    layer_count: 1,
                    ..Default::default()
                });

            let binding = [image_barrier];
            let dep_info = vk::DependencyInfo::default()
                .image_memory_barriers(&binding)
                .dependency_flags(vk::DependencyFlags::BY_REGION);

            unsafe { device.cmd_pipeline_barrier2(command_buffer, &dep_info) };
        }

        end_single_time_command(
            device,
            command_pool,
            instance.graphics_queue,
            command_buffer,
        );
    }

    pub fn generate_mipmaps(
        instance: &VulkanInstance,
        image: vk::Image,
        command_pool: vk::CommandPool,
        width: u32,
        height: u32,
        mip_level: u32,
    ) {
        let device = &instance.device;

        let command_buffer = begin_single_time_command(device, command_pool);

        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_array_layer(0)
            .layer_count(1)
            .level_count(1);

        let mut barrier = vk::ImageMemoryBarrier2::default()
            .image(image)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .subresource_range(subresource);

        let mut mip_width = width as i32;
        let mut mip_height = height as i32;

        for i in 1..mip_level {
            barrier.subresource_range.base_mip_level = i - 1;
            barrier.old_layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
            barrier.new_layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
            barrier.src_access_mask = vk::AccessFlags2::TRANSFER_WRITE;
            barrier.dst_access_mask = vk::AccessFlags2::TRANSFER_READ;
            barrier.src_stage_mask = vk::PipelineStageFlags2::TRANSFER;
            barrier.dst_stage_mask = vk::PipelineStageFlags2::TRANSFER;

            let binding = [barrier];
            let dep_info = vk::DependencyInfo::default()
                .image_memory_barriers(&binding)
                .dependency_flags(vk::DependencyFlags::BY_REGION);

            unsafe { device.cmd_pipeline_barrier2(command_buffer, &dep_info) };

            let src_subresource = vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .mip_level(i - 1)
                .base_array_layer(0)
                .layer_count(1);

            let dst_subresource = vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .mip_level(i)
                .base_array_layer(0)
                .layer_count(1);

            let blit = vk::ImageBlit::default()
                .src_offsets([
                    vk::Offset3D::default(),
                    vk::Offset3D {
                        x: mip_width,
                        y: mip_height,
                        z: 1,
                    },
                ])
                .src_subresource(src_subresource)
                .dst_offsets([
                    vk::Offset3D::default(),
                    vk::Offset3D {
                        x: (if mip_width > 1 { mip_width / 2 } else { 1 }),
                        y: (if mip_height > 1 { mip_height / 2 } else { 1 }),
                        z: 1,
                    },
                ])
                .dst_subresource(dst_subresource);

            unsafe {
                device.cmd_blit_image(
                    command_buffer,
                    image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[blit],
                    vk::Filter::LINEAR,
                )
            };

            barrier.old_layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;
            barrier.new_layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
            barrier.src_access_mask = vk::AccessFlags2::TRANSFER_READ;
            barrier.dst_access_mask = vk::AccessFlags2::SHADER_READ;
            barrier.src_stage_mask = vk::PipelineStageFlags2::TRANSFER;
            barrier.dst_stage_mask = vk::PipelineStageFlags2::FRAGMENT_SHADER;

            let binding = [barrier];
            let dep_info = vk::DependencyInfo::default()
                .image_memory_barriers(&binding)
                .dependency_flags(vk::DependencyFlags::BY_REGION);

            unsafe { device.cmd_pipeline_barrier2(command_buffer, &dep_info) };

            if mip_width > 1 {
                mip_width /= 2;
            }

            if mip_height > 1 {
                mip_height /= 2;
            }
        }

        barrier.subresource_range.base_mip_level = mip_level - 1;
        barrier.old_layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
        barrier.new_layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        barrier.src_access_mask = vk::AccessFlags2::TRANSFER_WRITE;
        barrier.dst_access_mask = vk::AccessFlags2::SHADER_READ;
        barrier.src_stage_mask = vk::PipelineStageFlags2::BLIT;
        barrier.dst_stage_mask = vk::PipelineStageFlags2::FRAGMENT_SHADER;

        let binding = [barrier];
        let dep_info = vk::DependencyInfo::default()
            .image_memory_barriers(&binding)
            .dependency_flags(vk::DependencyFlags::BY_REGION);

        unsafe { device.cmd_pipeline_barrier2(command_buffer, &dep_info) };

        end_single_time_command(
            device,
            command_pool,
            instance.graphics_queue,
            command_buffer,
        );
    }

    fn create_image_view(
        image: vk::Image,
        device: &ash::Device,
        format: vk::Format,
        mip_level: u32,
        mask: vk::ImageAspectFlags,
    ) -> vk::ImageView {
        let image_view_info = vk::ImageViewCreateInfo::default()
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(mask)
                    .level_count(mip_level)
                    .layer_count(1),
            )
            .image(image)
            .format(format)
            .view_type(vk::ImageViewType::TYPE_2D);

        unsafe { device.create_image_view(&image_view_info, None) }.unwrap()
    }

    // Do not cache, Every image mostly unique memory
    fn create_image(
        device: &ash::Device,
        format: vk::Format,
        size: Extent2D,
        mip_level: u32,
        usage: vk::ImageUsageFlags,
    ) -> vk::Image {
        let create_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(size.into())
            .mip_levels(mip_level)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        unsafe { device.create_image(&create_info, None) }.unwrap()
    }

    pub fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            device.destroy_image_view(self.image_view, None);
            device.destroy_image(self.image, None);
            //  device.destroy_sampler(self.sampler, None); Will be destroyed by the cache
            device.free_memory(self.memory, None);
        }
    }
}
