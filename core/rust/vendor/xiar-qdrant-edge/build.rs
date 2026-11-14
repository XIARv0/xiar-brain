// MODIFIED by XIAR: upstream also compiled C SIMD kernels here via `cc`; those are now pure Rust
// (see src/quantization/kernels.rs). Only the cfg detection from build_common remains.
mod build_common;

fn main() {
    build_common::main();
}
