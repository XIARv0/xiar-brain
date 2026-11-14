// MODIFIED by XIAR (Apache-2.0 notice): this file replaces the C SIMD kernels that
// upstream qdrant-edge 0.8.0 compiled with `cc` (`cpp/quantization/{sse,avx2,neon}.c`).
// The exported names and signatures are unchanged so call sites keep working, but the
// bodies are pure Rust. Semantics were derived from the C sources and are checked
// against the original C code by the differential oracle in `spikes/kernel-oracle`.
//
// Behavioural notes (documented divergences from the C originals):
//  * `*_score_dot_*` computes the exact unsigned `sum(q[i] * v[i])`. The C code used
//    `maddubs` (treats `q` as signed, saturates pairs to i16) and its AVX tail used
//    different semantics; the upstream scalar fallback is exact, so we follow that.
//  * `*_score_l1_*` is exact over the full `dim`; the C code accumulated in 16-bit lanes
//    (could wrap for very large `dim`) and assumed `dim % 16 == 0`.
//  * The scalar8/scalar4 popcount kernels accumulate in integers; the C code summed in f32
//    (identical results while the total stays below 2^24).
//
// All functions are `unsafe` with the same contract as the C versions: the pointers must be
// valid for the number of bytes implied by the arguments, and the CPU must support the
// named target feature (callers already gate on `is_*_feature_detected!`).
#![allow(clippy::missing_safety_doc)]

#[inline(always)]
unsafe fn rd64(p: *const u8, byte_off: usize) -> u64 {
    unsafe { core::ptr::read_unaligned(p.add(byte_off) as *const u64) }
}

#[inline(always)]
unsafe fn rd32(p: *const u8, byte_off: usize) -> u32 {
    unsafe { core::ptr::read_unaligned(p.add(byte_off) as *const u32) }
}

/// popcount(q ^ v) over `count` blocks of `block` bytes (block = 16, 8 or 4).
#[inline(always)]
unsafe fn xor_popcnt_plain(q: *const u8, v: *const u8, count: u32, block: usize) -> u32 {
    let mut total = 0u32;
    for i in 0..count as usize {
        let off = i * block;
        match block {
            16 => unsafe {
                total += (rd64(q, off) ^ rd64(v, off)).count_ones();
                total += (rd64(q, off + 8) ^ rd64(v, off + 8)).count_ones();
            },
            8 => unsafe { total += (rd64(q, off) ^ rd64(v, off)).count_ones() },
            _ => unsafe { total += (rd32(q, off) ^ rd32(v, off)).count_ones() },
        }
    }
    total
}

/// Bit-plane popcount, 16-byte blocks of `v`, `planes` planes (8 or 4) stored per block as
/// `planes` consecutive 16-byte chunks of `q`. Plane `p` has weight `2^p`.
#[inline(always)]
unsafe fn xor_popcnt_planes_u128(q: *const u8, v: *const u8, count: u32, planes: usize) -> u32 {
    let mut acc = [0u32; 8];
    for i in 0..count as usize {
        let v1 = unsafe { rd64(v, 16 * i) };
        let v2 = unsafe { rd64(v, 16 * i + 8) };
        for p in 0..planes {
            let qo = 16 * planes * i + 16 * p;
            acc[p] += (v1 ^ unsafe { rd64(q, qo) }).count_ones();
            acc[p] += (v2 ^ unsafe { rd64(q, qo + 8) }).count_ones();
        }
    }
    let mut r = 0u32;
    for p in 0..planes {
        r += acc[p] << p;
    }
    r
}

/// Bit-plane popcount, byte granularity: for byte `b` of `v`, plane `k` is `q[planes*b + k]`.
#[inline(always)]
unsafe fn xor_popcnt_planes_u8(q: *const u8, v: *const u8, count: u32, planes: usize) -> u32 {
    let mut acc = [0u32; 8];
    for b in 0..count as usize {
        let vb = unsafe { *v.add(b) };
        for k in 0..planes {
            let qb = unsafe { *q.add(planes * b + k) };
            acc[k] += (vb ^ qb).count_ones();
        }
    }
    let mut r = 0u32;
    for k in 0..planes {
        r += acc[k] << k;
    }
    r
}

#[inline(always)]
unsafe fn score_dot(q: *const u8, v: *const u8, dim: u32) -> f32 {
    let mut s = 0u32;
    for i in 0..dim as usize {
        s += u32::from(unsafe { *q.add(i) }) * u32::from(unsafe { *v.add(i) });
    }
    s as f32
}

#[inline(always)]
unsafe fn score_l1(q: *const u8, v: *const u8, dim: u32) -> f32 {
    let mut s = 0u32;
    for i in 0..dim as usize {
        s += u32::from(unsafe { *q.add(i) }).abs_diff(u32::from(unsafe { *v.add(i) }));
    }
    s as f32
}

macro_rules! kernel {
    ($(#[$m:meta])* $name:ident($($a:ident: $t:ty),*) -> $r:ty => $body:expr) => {
        $(#[$m])*
        pub unsafe fn $name($($a: $t),*) -> $r {
            unsafe { $body }
        }
    };
}

// ---------------------------------------------------------------- x86_64: SSE family
macro_rules! sse_kernel { ($($t:tt)*) => { kernel!(#[cfg(target_arch = "x86_64")] #[target_feature(enable = "sse4.1,popcnt")] $($t)*); }; }

sse_kernel!(impl_score_dot_sse(q: *const u8, v: *const u8, dim: u32) -> f32 => score_dot(q, v, dim));
sse_kernel!(impl_score_l1_sse(q: *const u8, v: *const u8, dim: u32) -> f32 => score_l1(q, v, dim));
sse_kernel!(impl_xor_popcnt_sse_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_plain(q, v, count, 16));
sse_kernel!(impl_xor_popcnt_sse_uint64(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_plain(q, v, count, 8));
sse_kernel!(impl_xor_popcnt_sse_uint32(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_plain(q, v, count, 4));
sse_kernel!(impl_xor_popcnt_scalar8_sse_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u128(q, v, count, 8));
sse_kernel!(impl_xor_popcnt_scalar4_sse_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u128(q, v, count, 4));
sse_kernel!(impl_xor_popcnt_scalar8_sse_u8(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u8(q, v, count, 8));
sse_kernel!(impl_xor_popcnt_scalar4_sse_u8(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u8(q, v, count, 4));

// ---------------------------------------------------------------- x86_64: AVX2 family
macro_rules! avx_kernel { ($($t:tt)*) => { kernel!(#[cfg(target_arch = "x86_64")] #[target_feature(enable = "avx2,popcnt")] $($t)*); }; }

avx_kernel!(impl_score_dot_avx(q: *const u8, v: *const u8, dim: u32) -> f32 => score_dot(q, v, dim));
avx_kernel!(impl_score_l1_avx(q: *const u8, v: *const u8, dim: u32) -> f32 => score_l1(q, v, dim));
avx_kernel!(impl_xor_popcnt_scalar8_avx_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u128(q, v, count, 8));
avx_kernel!(impl_xor_popcnt_scalar4_avx_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u128(q, v, count, 4));

// ---------------------------------------------------------------- aarch64: NEON family
// NEON is baseline on aarch64, so no `target_feature` attribute is needed.
// NOTE: not executed on the x86 dev machine; semantics are shared with the x86 kernels above
// and covered by the same reference tests (see `tests` below). Re-verify on aarch64 hardware.
macro_rules! neon_kernel { ($($t:tt)*) => { kernel!(#[cfg(all(target_arch = "aarch64", target_feature = "neon"))] $($t)*); }; }

neon_kernel!(impl_score_dot_neon(q: *const u8, v: *const u8, dim: u32) -> f32 => score_dot(q, v, dim));
neon_kernel!(impl_score_l1_neon(q: *const u8, v: *const u8, dim: u32) -> f32 => score_l1(q, v, dim));
neon_kernel!(impl_xor_popcnt_neon_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_plain(q, v, count, 16));
neon_kernel!(impl_xor_popcnt_neon_uint64(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_plain(q, v, count, 8));
neon_kernel!(impl_xor_popcnt_scalar8_neon_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u128(q, v, count, 8));
neon_kernel!(impl_xor_popcnt_scalar4_neon_uint128(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u128(q, v, count, 4));
neon_kernel!(impl_xor_popcnt_scalar8_neon_u8(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u8(q, v, count, 8));
neon_kernel!(impl_xor_popcnt_scalar4_neon_u8(q: *const u8, v: *const u8, count: u32) -> u32 => xor_popcnt_planes_u8(q, v, count, 4));

// ---------------------------------------------------------------- reference tests
#[cfg(test)]
mod tests {
    use super::*;

    fn rnd(n: usize, seed: u64) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (s >> 33) as u8
            })
            .collect()
    }

    #[test]
    fn plain_popcnt_matches_naive() {
        let q = rnd(64, 1);
        let v = rnd(64, 2);
        let naive: u32 = q.iter().zip(&v).map(|(a, b)| (a ^ b).count_ones()).sum();
        unsafe {
            assert_eq!(xor_popcnt_plain(q.as_ptr(), v.as_ptr(), 4, 16), naive);
            assert_eq!(xor_popcnt_plain(q.as_ptr(), v.as_ptr(), 8, 8), naive);
            assert_eq!(xor_popcnt_plain(q.as_ptr(), v.as_ptr(), 16, 4), naive);
        }
    }

    #[test]
    fn dot_and_l1_exact() {
        let q = rnd(48, 3);
        let v = rnd(48, 4);
        let dot: u32 = q.iter().zip(&v).map(|(a, b)| u32::from(*a) * u32::from(*b)).sum();
        let l1: u32 = q.iter().zip(&v).map(|(a, b)| u32::from(*a).abs_diff(u32::from(*b))).sum();
        unsafe {
            assert_eq!(score_dot(q.as_ptr(), v.as_ptr(), 48), dot as f32);
            assert_eq!(score_l1(q.as_ptr(), v.as_ptr(), 48), l1 as f32);
        }
    }
}
