# xiar-qdrant-edge: modified fork of qdrant-edge 0.8.0

- Upstream: qdrant-edge 0.8.0, https://github.com/qdrant/qdrant
- Copyright Qdrant Solutions GmbH. Licensed under the Apache License, Version 2.0 (see `LICENSE` in this folder).
- This folder stays under Apache-2.0. It is NOT relicensed under the AGPL; it is combined with the AGPL-licensed
  XIAR code as a separate, permissively licensed component.

Modifications by the XIAR project (copyright holders: see the top-level NOTICE):
- C SIMD kernels rewritten in Rust;
- tonic/gRPC and jieba (zstd) dependencies removed;
- storage page/chunk/WAL preallocation sizes reduced;
- package renamed `xiar-qdrant-edge`.
