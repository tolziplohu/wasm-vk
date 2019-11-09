# wasm-vk
`wasm-vk` is a command-line tool and Rust library to transpile WebAssembly into Vulkan SPIR-V.
It uses [`parity-wasm`](https://crates.io/crates/parity-wasm) to parse WASM and represent it internally,
and [`rspirv`](https://crates.io/crates/rspirv) to emit SPIR-V.
It makes no attempt to produce *optimized* SPIR-V - using spirv-opt after is probably a good idea.

# Why
WebAssembly was never meant just for the Web, it's meant to be used on many different platforms.
To that end, it doesn't have native support for most things, and requires an *embedder*.
It's also very simple, and only relies on functionality that's very well supported across architectures.

Because of all this, we can create a WebAssembly embedder that runs on the GPU, and we can support almost all of WebAssembly.

# Mapping
`wasm-vk` compiles a WebAssembly module into a Vulkan compute shader, currently with local size hardcoded as 64x1x1.
WASM's *linear memory* is represented as a single Vulkan buffer, at descriptor set 0 and binding 0, which can be both read and written to by the shader.
It uses the module's start function as the entry point, and shaders can define a global i32 "spv.id" which represents the thread index (gl_GlobalInvocationID.x, specifically).
See `examples/comp.wat` for an example of a compute shader written in WebAssembly.

We'll eventually add imports for other SPIR-V builtins, and we may use the atomic operations from the WebAssembly threads proposal, and probably force the memory to be marked `shared`.

# Usage
### Command-line usage
```
wasm-vk [options] <input.wasm> [output.spv]

If no output file is given, it will default to 'out.spv'.

Options:
-v, --verbose       Show more output, including dissasembled SPIR-V
-h, --help          Show this help
```

### Library usage
wasm-vk isn't on crates.io yet, but you can try it if you want.
You probably want to use `wasm_vk::spirv::to_spirv`, which you can pass a `parity-wasm` `Module`.
We re-export `deserialize` and `deserialize_file` from `parity-wasm`, but you can also create a `parity-wasm` `Module` yourself.

Note that wasm-vk doesn't interact at all with Vulkan - it just produces SPIR-V bytes for use with any Vulkan library.
See `examples/vulkano.rs` for an example using [`Vulkano`](https://crates.io/crates/vulkano) to load and run a WebAssembly compute shader.

# Current status
See `examples/comp.wat` for everything `wasm-vk` currently supports.
Supported instructions:
```
- i32.mul
- i32.add
- i32.load
- i32.store
- i32.const
- local.get (just for i32s)
- local.set (just for i32s)
- i32.eq
- i32.le_u
- if/then/else/end
- loop
- br / br_if (works for loops, sometimes works for blocks and ifs - keep your 'br' indices low for the best chance)
- return (without a value)
- global.get (just for imported globals like 'spv.id')
```
