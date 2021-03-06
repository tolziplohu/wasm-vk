use vulkano::buffer::{BufferUsage, CpuAccessibleBuffer};
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::descriptor::descriptor_set::PersistentDescriptorSet;
use vulkano::device::{Device, DeviceExtensions};
use vulkano::instance::{Instance, InstanceExtensions, PhysicalDevice};
use vulkano::pipeline::ComputePipeline;
use vulkano::sync;
use vulkano::sync::GpuFuture;

use wasm_vk::*;

use std::sync::Arc;

const BUFFER_SIZE: usize = 65536;

/// Loads, parses, and validates a WAT file, then passes it to `run_module`
#[test]
fn two_buffers() -> std::io::Result<()> {
    use std::fs::File;
    use std::io::prelude::*;
    use std::io::BufReader;
    use std::path::PathBuf;

    let test = "two_buffers.wat";
    let test: PathBuf = ["tests", &test].iter().collect();
    let test_name = test.file_name().unwrap().to_str().unwrap().to_string();
    let f = File::open(test)?;
    let mut buf_reader = BufReader::new(f);

    let expected: Vec<u32> = vec![16, 13, 15, 17, 19, 21, 18];

    let mut buf = Vec::new();
    buf_reader.read_to_end(&mut buf)?;
    match wabt::wat2wasm(buf) {
        Ok(binary) => {
            let w = wasm::deserialize_buffer(&binary).unwrap();
            let got = run_module(w);
            if got[..expected.len()] == *expected {
                println!("Test {} passed", test_name);
                Ok(())
            } else {
                eprintln!(
                    "Test {} failed, expected {:?}, got {:?}",
                    test_name,
                    expected,
                    &got[..expected.len()]
                );
                Err(std::io::Error::from(std::io::ErrorKind::Other))
            }
        }
        Err(e) => {
            eprintln!("Test {} failed verification: {:?}", test_name, e);
            Err(std::io::Error::from(std::io::ErrorKind::Other))
        }
    }
}

/// Runs a module in Vulkano. Segfaults if the generated SPIR-V isn't valid.
/// Note that if generated SPIR-V isn't valid for one test, the segfault will still abort the whole test process,
/// so it will look like all tests failed.
fn run_module(w: wasm::Module) -> Vec<u32> {
    // First, we generate SPIR-V
    let ctx = spirv::Ctx::new();
    let m = ctx.module(&w);
    println!("{}", {
        use rspirv::binary::Disassemble;
        m.disassemble()
    });
    let spv = spirv::module_bytes(m);

    // Here's the data we'll be using, it's just BUFFER_SIZE consecutive u32s, starting at 0
    let data_iter = 0..BUFFER_SIZE as u32;
    // The other buffer
    // [16, 12, 13, 14, 15, 16, 12, ..]
    let data_2_iter = (0..BUFFER_SIZE as u32).map(|x| 12 + (x + 4) % 5);

    // Now we'll run the SPIR-V on the GPU with Vulkano.
    // This is a bunch of boilerplate, see the Vulkano examples for explanations.

    let instance = Instance::new(None, &InstanceExtensions::none(), None).unwrap();
    let physical = PhysicalDevice::enumerate(&instance).next().unwrap();
    let queue_family = physical
        .queue_families()
        .find(|&q| q.supports_compute())
        .unwrap();

    let (device, mut queues) = Device::new(
        physical,
        physical.supported_features(),
        &DeviceExtensions::none(),
        [(queue_family, 0.5)].iter().cloned(),
    )
    .unwrap();

    let queue = queues.next().unwrap();

    // This is pretty messy, but is pretty much what you need to do to get your own SPIR-V loaded with Vulkano
    let pipeline = Arc::new({
        #[derive(Copy, Clone)]
        struct PLayout;
        unsafe impl vulkano::descriptor::pipeline_layout::PipelineLayoutDesc for PLayout {
            fn num_sets(&self) -> usize {
                1
            }
            fn num_bindings_in_set(&self, set: usize) -> Option<usize> {
                assert_eq!(set, 0);
                Some(2)
            }
            fn descriptor(
                &self,
                set: usize,
                _binding: usize,
            ) -> Option<vulkano::descriptor::descriptor::DescriptorDesc> {
                assert_eq!(self.num_bindings_in_set(set), Some(2));
                // Both are the same
                Some(vulkano::descriptor::descriptor::DescriptorDesc {
                    ty: vulkano::descriptor::descriptor::DescriptorDescTy::Buffer(
                        vulkano::descriptor::descriptor::DescriptorBufferDesc {
                            // I have no idea what these do
                            dynamic: Some(false),
                            storage: true,
                        },
                    ),
                    array_count: 1,
                    stages: vulkano::descriptor::descriptor::ShaderStages::compute(),
                    readonly: false,
                })
            }
            fn num_push_constants_ranges(&self) -> usize {
                0
            }
            fn push_constants_range(
                &self,
                _num: usize,
            ) -> Option<vulkano::descriptor::pipeline_layout::PipelineLayoutDescPcRange>
            {
                None
            }
        }

        let shader =
            unsafe { vulkano::pipeline::shader::ShaderModule::new(device.clone(), &spv).unwrap() };

        let entry_str = std::ffi::CString::new("main").unwrap();

        let entry = unsafe { shader.compute_entry_point(&entry_str, PLayout) };

        ComputePipeline::new(device.clone(), &entry, &()).unwrap()
    });

    let data_buffer =
        CpuAccessibleBuffer::from_iter(device.clone(), BufferUsage::all(), data_iter.clone())
            .unwrap();
    let data_2_buffer =
        CpuAccessibleBuffer::from_iter(device.clone(), BufferUsage::all(), data_2_iter.clone())
        .unwrap();

    let set = Arc::new(
        PersistentDescriptorSet::start(pipeline.clone(), 0)
            .add_buffer(data_buffer.clone())
            .unwrap()
            .add_buffer(data_2_buffer.clone())
            .unwrap()
            .build()
            .unwrap(),
    );

    let command_buffer =
        AutoCommandBufferBuilder::primary_one_time_submit(device.clone(), queue.family())
            .unwrap()
            .dispatch([1024, 1, 1], pipeline.clone(), set.clone(), ())
            .unwrap()
            // Finish building the command buffer by calling `build`.
            .build()
            .unwrap();

    let future = sync::now(device.clone())
        .then_execute(queue.clone(), command_buffer)
        .unwrap()
        .then_signal_fence_and_flush()
        .unwrap();

    future.wait(None).unwrap();

    // Here's the data the GPU got
    let b = data_buffer.read().unwrap();
    b.to_vec()
}
