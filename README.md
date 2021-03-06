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
WASM modules define readable and writeable buffers with specially named imports of load and store functions, for example "buffer:0:2:load" for a buffer at set=0 and binding=2.
It uses the module's start function as the entry point, and shaders can define a global i32 "spv.id" which represents the thread index (gl_GlobalInvocationID.x, specifically).
We'll eventually add imports for other SPIR-V builtins.

See `examples/comp.wat` for an example of a compute shader written in WebAssembly, or `examples/image.wat` for one written in Rust and compiled to WebAssembly.

## Linear memory
We emulate a heap for linear memory with a stack-allocated array if the WASM module needs it.
It's always exactly 128 bytes in size - it panics if the data section is bigger than that, but has undefined behaviour if a load or store in the shader goes over.
We're somewhat intelligent about which 128 bytes to use, though. If a data section is present, the data section is in the middle of the 128 byte window.
Otherwise, it starts 64 bytes before the pointer passed to the first load or store.
That's enough to work with LLVM's bump-down stack allocator in most cases.

Note that only loads and stores aligned to 4-byte boundaries will work currently.

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
`wasm-vk` isn't on crates.io yet, but you can try it if you want.
Note that it doesn't interact at all with Vulkan - it just produces SPIR-V bytes for use with any Vulkan library.
See `examples/vulkano.rs` for an example using [`Vulkano`](https://crates.io/crates/vulkano) to load and run a WebAssembly compute shader.
See `examples/image.rs` for an example of generating an image in a compute shader.

```rust
use wasm_vk::*;

// The type annotations make it clearer what everything returns
// Grab the raw WASM from a file
let w: wasm::Module = wasm::deserialize_file("examples/comp.wasm").unwrap();

let ctx = spirv::Ctx::new();
// This translates it to wasm-vk's IR and then to SPIR-V
let m: spirv::Module = ctx.module(&w);

// Assemble the SPIR-V to get the bytes for use with Vulkan
let spv: Vec<u8> = spirv::module_bytes(m);
```

# Current status
See `examples/comp.wat` for most of what `wasm-vk` currently supports.
Supported instructions:
```
General operations:
- nop
- global.get (just for 'spv.id' builtin)
- local.set
- local.get
- local.tee
Numeric operations: All i32 and f32 instructions EXCEPT:
- i32.clz
- i32.ctz
- i32.popcnt
- i32.rem_*
- i32.rotr
- i32.rotl
- f32.trunc
- f32.nearest
- f32.copysign
- reinterpret instructions
Control flow (note: currently only if's can have types attached, and br's can't have a value):
- select
- loop
- block
- if/then/else
- br
- br_if
- return (without a value)
- call (we support functions in general)
```
