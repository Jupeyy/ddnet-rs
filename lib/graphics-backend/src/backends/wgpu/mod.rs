use std::sync::{atomic::AtomicU64, Arc};

use anyhow::anyhow;
use graphics_backend_traits::{
    plugin::BackendCustomPipeline,
    traits::{DriverBackendInterface, GraphicsBackendMtInterface},
};
use graphics_types::{
    commands::AllCommands,
    types::{
        GraphicsBackendMemory, GraphicsBackendMemoryAllocation, GraphicsMemoryAllocationMode,
        GraphicsMemoryAllocationType,
    },
};
use hiarc::Hiarc;

use crate::window::BackendDisplayRequirements;

use super::{types::BackendWriteFiles, vulkan::Options};

type ArcRwLock<T> = Arc<parking_lot::RwLock<T>>;

#[derive(Debug, Hiarc)]
pub struct WgpuMainThreadData {
    instance: wgpu::Instance,
    phy_gpu: wgpu::Device,
    mem_allocator: Arc<parking_lot::Mutex<VulkanAllocator>>,
}

#[derive(Debug)]
pub struct WgpuLoading {}

impl WgpuLoading {
    pub fn new(
        display_requirements: BackendDisplayRequirements,
        texture_memory_usage: Arc<AtomicU64>,
        buffer_memory_usage: Arc<AtomicU64>,
        stream_memory_usage: Arc<AtomicU64>,
        staging_memory_usage: Arc<AtomicU64>,

        options: &Options,

        custom_pipes: Option<ArcRwLock<Vec<Box<dyn BackendCustomPipeline>>>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {})
    }
}

#[derive(Debug, Hiarc)]
pub struct Wgpu {}

impl Wgpu {
    pub fn new(
        mut loading: WgpuLoading,
        loaded_io: VulkanBackendLoadedIo,
        runtime_threadpool: &Arc<rayon::ThreadPool>,

        main_thread_data: VulkanMainThreadInit,
        window_width: u32,
        window_height: u32,
        options: &Options,

        write_files: BackendWriteFiles,
    ) -> anyhow::Result<Box<Self>> {
        Ok(Box::new(Self {}))
    }

    pub fn get_mt_backend() -> WgpuBackendMt {
        WgpuBackendMt {}
    }

    pub fn main_thread_data(loading: &WgpuLoading) -> WgpuMainThreadData {
        let instance = loading.props.ash_vk.vk_device.phy_device.instance.clone();
        let phy_gpu = loading.props.ash_vk.vk_device.phy_device.clone();
        let mem_allocator = loading.props.device.mem_allocator.clone();
        WgpuMainThreadData {
            instance,
            mem_allocator,
            phy_gpu,
        }
    }
}

impl DriverBackendInterface for Wgpu {
    fn attach_frame_fetcher(
        &mut self,
        _name: String,
        _fetcher: std::sync::Arc<
            dyn graphics_backend_traits::frame_fetcher_plugin::BackendFrameFetcher,
        >,
    ) {
        // do nothing
    }

    fn detach_frame_fetcher(&mut self, _name: String) {
        // do nothing
    }

    fn run_command(&mut self, _cmd: AllCommands) -> anyhow::Result<()> {
        // nothing to do
        Ok(())
    }

    fn start_commands(&mut self, _command_count: usize) {
        // nothing to do
    }

    fn end_commands(&mut self) -> anyhow::Result<()> {
        // nothing to do
        Ok(())
    }
}

pub fn mem_alloc_lazy(alloc_type: GraphicsMemoryAllocationType) -> GraphicsBackendMemory {
    let mut mem: Vec<u8> = Default::default();
    match alloc_type {
        GraphicsMemoryAllocationType::TextureRgbaU8 { width, height, .. } => {
            mem.resize(width.get() * height.get() * 4, Default::default())
        }
        GraphicsMemoryAllocationType::TextureRgbaU82dArray {
            width,
            height,
            depth,
            ..
        } => mem.resize(
            width.get() * height.get() * depth.get() * 4,
            Default::default(),
        ),
        GraphicsMemoryAllocationType::Buffer { required_size } => {
            mem.resize(required_size.get(), Default::default())
        }
    }
    GraphicsBackendMemory::new(GraphicsBackendMemoryAllocation::Vector(mem), alloc_type)
}

#[derive(Debug, Hiarc)]
pub struct WgpuBackendMt {}

impl GraphicsBackendMtInterface for WgpuBackendMt {
    fn mem_alloc(
        &self,
        alloc_type: GraphicsMemoryAllocationType,
        _mode: GraphicsMemoryAllocationMode,
    ) -> GraphicsBackendMemory {
        mem_alloc_lazy(alloc_type)
    }

    fn try_flush_mem(
        &self,
        _mem: &mut GraphicsBackendMemory,
        _do_expensive_flushing: bool,
    ) -> anyhow::Result<()> {
        Err(anyhow!("this operation is not supported."))
    }
}
