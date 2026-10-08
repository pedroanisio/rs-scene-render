//! Stable radix-sort equivalence on the actual adapter, including ties and partial blocks.
use super::*;

fn sorted(eng: &ThreeEngine, keys: &[u32], scatter: &str) -> Vec<u32> {
    sorted_timed(eng, keys, scatter).0
}
fn sorted_timed(eng: &ThreeEngine, keys: &[u32], scatter: &str) -> (Vec<u32>, f64) {
    let d = &eng.device;
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(sort_src().into()),
    });
    let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&eng.bgl_sort)],
        immediate_size: 0,
    });
    let scatter = {
        let _creation = crate::gpu::creation_lock();
        d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&layout),
            module: &module,
            entry_point: Some(scatter),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let n = keys.len() as u32;
    let blocks = n.max(1).div_ceil(256);
    let storage = |v: &[u32]| {
        d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(v),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        })
    };
    let input: Vec<u32> = if keys.is_empty() { vec![0] } else { keys.to_vec() };
    let (ka, kb) = (storage(&input), storage(&vec![0; input.len()]));
    let (va, vb) = (storage(&(0..n.max(1)).collect::<Vec<_>>()), storage(&vec![0; input.len()]));
    let hist = storage(&vec![0; blocks as usize * 256]);
    let dummy = storage(&[0; 16]);
    let mut bytes = Vec::new();
    for pass in 0..4 {
        pad(
            &mut bytes,
            bytemuck::bytes_of(&SortU { n, blocks, shift: pass * 8, pad: 0, view: Mat4::IDENTITY.to_cols_array_2d() }),
        );
    }
    let params = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind = |ki: &wgpu::Buffer, vi: &wgpu::Buffer, ko: &wgpu::Buffer, vo: &wgpu::Buffer| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &eng.bgl_sort,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &params,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<SortU>() as u64),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: dummy.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: ki.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: vi.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: ko.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: vo.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: hist.as_entire_binding() },
            ],
        })
    };
    let (ab, ba) = (bind(&ka, &va, &kb, &vb), bind(&kb, &vb, &ka, &va));
    let mut enc = d.create_command_encoder(&Default::default());
    {
        let mut cp = enc.begin_compute_pass(&Default::default());
        for pass in 0..4 {
            cp.set_bind_group(0, if pass % 2 == 0 { &ab } else { &ba }, &[pass * 256]);
            cp.set_pipeline(&eng.sort_pipes[1]);
            cp.dispatch_workgroups(blocks, 1, 1);
            cp.set_pipeline(&eng.sort_pipes[2]);
            cp.dispatch_workgroups(1, 1, 1);
            cp.set_pipeline(&scatter);
            cp.dispatch_workgroups(blocks, 1, 1);
        }
    }
    let read = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: input.len() as u64 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_buffer_to_buffer(&va, 0, &read, 0, input.len() as u64 * 4);
    let command = enc.finish();
    let clock = std::time::Instant::now();
    eng.queue.submit([command]);
    read.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let elapsed = clock.elapsed().as_secs_f64();
    let view = read.slice(..).get_mapped_range().unwrap();
    (bytemuck::cast_slice::<u8, u32>(&view)[..n as usize].to_vec(), elapsed)
}

#[test]
fn software_scatter_matches_shader_and_stable_reference() {
    let gpu = match crate::gpu::test_gpu() {
        Ok(g) => g,
        Err(e) => {
            assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "{e}");
            return;
        }
    };
    let eng = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    for n in [0, 1, 255, 256, 257, 1025] {
        let mut seed = 7u32;
        let keys: Vec<u32> = (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                if i % 7 == 0 {
                    u32::MAX
                } else if i % 3 == 0 {
                    42
                } else {
                    seed
                }
            })
            .collect();
        let mut expected: Vec<u32> = (0..n).collect();
        expected.sort_by_key(|&i| keys[i as usize]);
        assert_eq!(sorted(&eng, &keys, "cs_scatter"), expected, "original shader, n={n}");
        assert_eq!(sorted(&eng, &keys, "cs_scatter_cpu"), expected, "software shader, n={n}");
    }
}

#[test]
#[ignore = "Actual-adapter radix-sort benchmark; run with --ignored --nocapture"]
fn benchmark_software_scatter() {
    let gpu = crate::gpu::test_gpu().unwrap();
    let eng = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
    let keys: Vec<u32> = (0..262145u32).map(|i| i.wrapping_mul(2654435761)).collect();
    let expected = sorted(&eng, &keys, "cs_scatter");
    let mut times = [Vec::new(), Vec::new()];
    for round in 0..6 {
        for slot in 0..2 {
            let i = (round + slot) % 2;
            let (got, t) = sorted_timed(&eng, &keys, if i == 0 { "cs_scatter" } else { "cs_scatter_cpu" });
            assert_eq!(got, expected);
            times[i].push(t * 1000.);
        }
    }
    for (name, mut times) in ["original scatter", "software scatter"].into_iter().zip(times) {
        times.sort_by(f64::total_cmp);
        println!("{name}: median {:.3} ms; {times:?}", (times[2] + times[3]) * 0.5);
    }
}
