<!-- SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com> -->
<!-- SPDX-License-Identifier: CC-BY-4.0 -->

# Third-party licences

XIAR's own code in this repository is AGPL-3.0-or-later with the §7(b) "Powered by XIAR V0" term (see
`LICENSE`, `LICENSE-ADDITIONAL-TERMS.md`, `NOTICE`). Third-party material keeps its own licence. Every
licence listed below is permissive and may be combined into an AGPL-3.0 work, unless flagged.

Generated 2026-11-14 (America/New_York) from `core/rust/Cargo.lock` with `cargo tree --target all`
(licence strings as declared by each crate's `Cargo.toml`) and from the Python/JS imports. **Before any
public release, regenerate this from an SBOM (plan §4A.10) and include each licence's full text where the
licence requires it (MIT/BSD/ISC/Apache-2.0 notices travel with binary releases).**

## 1. Bundled third-party source in this repository

| Component | Path | Licence | Copyright / notes |
|---|---|---|---|
| **MIRIX** (modified / ported) | Rust port in `core/rust/crates/xiar-memory*`, `xiar-core`, `xiar-memory-api/src/legacy/timeutil.rs`; original prompts in `core/rust/prompts/mirix/` | **Apache-2.0** (verified from https://github.com/Mirix-AI/MIRIX/blob/main/LICENSE on 2026-11-14) | Copyright Mirix-AI and MIRIX contributors; upstream LICENSE also says "Copyright 2023, Letta authors". Ported from commits `8cb06a62` (main) and `f7e75fc3` (desktop-agent, incl. PR #92). The Rust port is XIAR code under AGPL-3.0-or-later (modified from MIRIX, notices kept); the original prompts stay Apache-2.0 with the licence text and an edit log in `core/rust/prompts/mirix/`. |
| **Letta** (formerly MemGPT), via MIRIX | — | Apache-2.0 | Credited out of caution because MIRIX's LICENSE names "Letta authors"; counsel to confirm the attribution chain. |
| **qdrant-edge 0.8.0** (modified fork) | `core/rust/vendor/xiar-qdrant-edge/` | Apache-2.0 | Copyright Qdrant Solutions GmbH. Changes listed in `NOTICE-XIAR.md` there. |
| **Mistral-7B-v0.1 tokenizer** (`tokenizer.json` only, no weights) | `core/neural-brain/tokenizers/mistral-v0.1/` | Apache-2.0 | Copyright Mistral AI; licence text included. |

## 2. Python dependencies (not bundled; installed by the user)

| Package | Used by | Licence |
|---|---|---|
| torch (PyTorch) | neural brain model/trainer | BSD-3-Clause (BSD-style) |
| tokenizers (Hugging Face) | neural brain tokenizer | Apache-2.0 |
| huggingface_hub | `xiar-brain fetch-tokenizer` | Apache-2.0 |
| fastapi | neural brain HTTP service | MIT |
| pydantic | neural brain request models | MIT |
| uvicorn | neural brain HTTP server | BSD-3-Clause |
| cryptography | sealed lesson store (X25519 + AES-GCM) | Apache-2.0 OR BSD-3-Clause |
| tiktoken | tokenizer benchmark only (`bench/`) | MIT |
| pytest | tests only | MIT |

The Hermes adapter (`integrations/hermes-xiar`) imports `hermes_constants`, `hermes_cli`, `run_agent`,
`model_tools` and `gateway` from **Hermes Agent** at runtime when installed. Hermes is **not** bundled; it is
an optional external runtime with its own licence [owner: confirm Hermes's licence before publishing the adapter].

## 3. JavaScript dependencies of the Swipe Deck view (not bundled)

| Package | Licence |
|---|---|
| react | MIT |
| react-router-dom | MIT |
| three (three.js) | MIT |

`GUI/src/settings-web/SwipeDeckView.tsx` also imports `./invoke` and `os/xiar-desktop/src/lib/desktopState`
from the full XIAR GUI, which are not part of this preview repo (reference copy only; it does not build alone).

## 4. Licences that need a note

- `self_cell` — `Apache-2.0 OR GPL-2.0-only`: used under **Apache-2.0** (GPL-2.0-only would be AGPL-incompatible).
- `r-efi` — `MIT OR Apache-2.0 OR LGPL-2.1-or-later`: used under MIT/Apache-2.0 (UEFI target only).
- `ryu` — `Apache-2.0 OR BSL-1.0`; `xxhash-rust` — `BSL-1.0` (Boost; GPL-compatible).
- `Unicode-3.0` (ICU data crates) — permissive, GPL-compatible.
- `webpki-root-certs` — CDLA-Permissive-2.0 (test TLS roots, not compiled into binaries).
- No GPL-only, AGPL-incompatible or proprietary crate is in the normal (shipped) dependency set.

## 5. Rust crates compiled into the workspace (normal + build dependencies, all targets)

| Crate | Version | Licence (declared) | Repository |
|---|---|---|---|
| addr2line | 0.27.1 | Apache-2.0 OR MIT | https://github.com/gimli-rs/addr2line |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | https://github.com/oyvindln/adler2 |
| aead | 0.5.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| aes | 0.8.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/block-ciphers |
| ahash | 0.8.12 | MIT OR Apache-2.0 | https://github.com/tkaitchuck/ahash |
| aho-corasick | 1.1.5 | Unlicense OR MIT | https://github.com/BurntSushi/aho-corasick |
| aligned-vec | 0.6.4 | MIT | https://github.com/sarah-ek/aligned-vec/ |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 | https://github.com/zakarumych/allocator-api2 |
| allocator-api2 | 0.4.0 | MIT OR Apache-2.0 | https://github.com/zakarumych/allocator-api2 |
| android_system_properties | 0.1.6 | MIT OR Apache-2.0 | https://github.com/nical/android_system_properties |
| anstream | 1.0.0 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle | 1.0.14 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-parse | 1.0.0 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| anstyle-wincon | 3.0.11 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| approx | 0.5.1 | Apache-2.0 | https://github.com/brendanzab/approx |
| argon2 | 0.5.3 | MIT OR Apache-2.0 | https://github.com/RustCrypto/password-hashes/tree/master/argon2 |
| arrayvec | 0.4.12 | MIT/Apache-2.0 | https://github.com/bluss/arrayvec |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 | https://github.com/bluss/arrayvec |
| async-broadcast | 0.7.2 | MIT OR Apache-2.0 | https://github.com/smol-rs/async-broadcast |
| async-channel | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-channel |
| async-executor | 1.14.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-executor |
| async-fs | 2.2.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-fs |
| async-io | 2.6.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-io |
| async-lock | 3.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-lock |
| async-process | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-process |
| async-recursion | 1.2.0 | MIT OR Apache-2.0 | https://github.com/dcchut/async-recursion |
| async-signal | 0.2.14 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-signal |
| async-task | 4.7.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/async-task |
| async-trait | 0.1.92 | MIT OR Apache-2.0 | https://github.com/dtolnay/async-trait |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | https://github.com/smol-rs/atomic-waker |
| atomic_refcell | 0.1.14 | Apache-2.0 OR MIT | https://github.com/mozilla/atomic_refcell |
| atomicwrites | 0.4.4 | MIT | https://github.com/untitaker/rust-atomicwrites |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | https://github.com/cuviper/autocfg |
| axum | 0.8.9 | MIT | https://github.com/tokio-rs/axum |
| axum-core | 0.5.6 | MIT | https://github.com/tokio-rs/axum |
| backtrace | 0.3.77 | MIT OR Apache-2.0 | https://github.com/rust-lang/backtrace-rs |
| base64 | 0.22.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| base64 | 0.23.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| base64ct | 1.8.3 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats |
| bincode | 1.3.3 | MIT | https://github.com/servo/bincode |
| bincode | 2.0.1 | MIT | https://github.com/bincode-org/bincode |
| bincode_derive | 2.0.1 | MIT | https://github.com/bincode-org/bincode |
| binout | 0.3.2 | MIT OR Apache-2.0 | https://github.com/beling/bsuccinct-rs |
| bit-vec | 0.6.3 | MIT/Apache-2.0 | https://github.com/contain-rs/bit-vec |
| bitflags | 1.3.2 | MIT/Apache-2.0 | https://github.com/bitflags/bitflags |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | https://github.com/bitflags/bitflags |
| bitm | 0.4.3 | MIT OR Apache-2.0 | https://github.com/beling/bsuccinct-rs |
| bitpacking | 0.9.3 | MIT | https://github.com/quickwit-oss/bitpacking |
| bitvec | 1.1.1 | MIT | https://github.com/bitvecto-rs/bitvec |
| blake2 | 0.10.6 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| blake2-rfc | 0.2.18 | MIT OR Apache-2.0 | https://github.com/cesarb/blake2-rfc |
| blink-alloc | 0.4.0 | MIT OR Apache-2.0 | https://github.com/zakarumych/blink-alloc |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| block-padding | 0.3.3 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| blocking | 1.7.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/blocking |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | https://github.com/fitzgen/bumpalo |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| byteorder | 1.5.0 | Unlicense OR MIT | https://github.com/BurntSushi/byteorder |
| bytes | 1.12.1 | MIT | https://github.com/tokio-rs/bytes |
| cbc | 0.1.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/block-modes |
| cc | 1.11.0 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/cfg-if |
| cfg_aliases | 0.2.2 | MIT | https://github.com/katharostech/cfg_aliases |
| cgroups-rs | 0.5.1 | MIT OR Apache-2.0 | https://github.com/kata-containers/cgroups-rs |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/stream-ciphers |
| chacha20 | 0.9.1 | Apache-2.0 OR MIT | https://github.com/RustCrypto/stream-ciphers |
| chacha20poly1305 | 0.10.1 | Apache-2.0 OR MIT | https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305 |
| charabia | 0.9.9 | MIT | https://github.com/meilisearch/charabia |
| chrono | 0.4.45 | MIT OR Apache-2.0 | https://github.com/chronotope/chrono |
| cipher | 0.4.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| colorchoice | 1.0.5 | MIT OR Apache-2.0 | https://github.com/rust-cli/anstyle.git |
| concurrent-queue | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/concurrent-queue |
| const-oid | 0.10.2 | Apache-2.0 OR MIT | https://github.com/RustCrypto/formats |
| constant_time_eq | 0.1.5 | CC0-1.0 | https://github.com/cesarb/constant_time_eq |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | https://github.com/servo/core-foundation-rs |
| core_affinity | 0.8.3 | MIT/Apache-2.0 | https://github.com/Elzair/core_affinity_rs |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| crc32c | 0.6.9 | MIT OR Apache-2.0 | https://github.com/zowens/crc32c |
| crc32fast | 1.5.4 | MIT OR Apache-2.0 | https://github.com/srijs/rust-crc32fast |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-deque | 0.8.8 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-epoch | 0.9.21 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crunchy | 0.2.4 | MIT | https://github.com/eira-fransham/crunchy |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| csv | 1.4.0 | Unlicense/MIT | https://github.com/BurntSushi/rust-csv |
| csv-core | 0.1.13 | Unlicense/MIT | https://github.com/BurntSushi/rust-csv |
| curve25519-dalek | 4.1.3 | BSD-3-Clause | https://github.com/dalek-cryptography/curve25519-dalek/tree/main/curve25519-dalek |
| curve25519-dalek-derive | 0.1.1 | MIT/Apache-2.0 | https://github.com/dalek-cryptography/curve25519-dalek |
| daachorse | 1.0.1 | MIT OR Apache-2.0 | https://github.com/daac-tools/daachorse |
| darling | 0.23.0 | MIT | https://github.com/TedDriggs/darling |
| darling_core | 0.23.0 | MIT | https://github.com/TedDriggs/darling |
| darling_macro | 0.23.0 | MIT | https://github.com/TedDriggs/darling |
| data-encoding | 2.11.1 | MIT | https://github.com/ia0/data-encoding |
| digest | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| digest | 0.11.3 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | https://github.com/yaahc/displaydoc |
| docopt | 1.1.1 | Unlicense/MIT | https://github.com/docopt/docopt.rs |
| duplicate | 2.0.1 | MIT OR Apache-2.0 | https://github.com/Emoun/duplicate |
| dyn-clone | 1.0.20 | MIT OR Apache-2.0 | https://github.com/dtolnay/dyn-clone |
| dyn_size_of | 0.4.5 | MIT OR Apache-2.0 | https://github.com/beling/bsuccinct-rs |
| earcut | 0.4.5 | ISC | https://github.com/ciscorn/earcut-rs |
| ecow | 0.3.1 | MIT OR Apache-2.0 | https://github.com/typst/ecow |
| ed25519 | 2.2.3 | Apache-2.0 OR MIT | https://github.com/RustCrypto/signatures/tree/master/ed25519 |
| ed25519-dalek | 2.2.0 | BSD-3-Clause | https://github.com/dalek-cryptography/curve25519-dalek/tree/main/ed25519-dalek |
| either | 1.19.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/either |
| endi | 1.1.1 | MIT | https://github.com/zeenix/endi |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 | https://github.com/meithecatte/enumflags2 |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 | https://github.com/meithecatte/enumflags2 |
| env_filter | 2.0.0 | MIT OR Apache-2.0 | https://github.com/rust-cli/env_logger |
| env_logger | 0.11.11 | MIT OR Apache-2.0 | https://github.com/rust-cli/env_logger |
| equator | 0.4.2 | MIT | https://github.com/sarah-ek/equator/ |
| equator-macro | 0.4.2 | MIT | https://github.com/sarah-ek/equator/ |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/equivalent |
| erased-serde | 0.4.11 | MIT OR Apache-2.0 | https://github.com/dtolnay/erased-serde |
| errno | 0.3.14 | MIT OR Apache-2.0 | https://github.com/lambda-fairy/rust-errno |
| event-listener | 5.4.2 | Apache-2.0 OR MIT | https://github.com/smol-rs/event-listener |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT | https://github.com/smol-rs/event-listener-strategy |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/fastrand |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | https://github.com/image-rs/fdeflate |
| fiat-crypto | 0.2.9 | MIT OR Apache-2.0 OR BSD-1-Clause | https://github.com/mit-plv/fiat-crypto |
| filetime | 0.2.29 | MIT/Apache-2.0 | https://github.com/alexcrichton/filetime |
| find-msvc-tools | 0.1.15 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| fixedbitset | 0.4.2 | MIT/Apache-2.0 | https://github.com/petgraph/fixedbitset |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| float_next_after | 2.0.0 | MIT | https://gitlab.com/bronsonbdevost/next_afterf |
| fnv | 1.0.7 | Apache-2.0 / MIT | https://github.com/servo/rust-fnv |
| foldhash | 0.1.5 | Zlib | https://github.com/orlp/foldhash |
| foldhash | 0.2.0 | Zlib | https://github.com/orlp/foldhash |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| fs-err | 3.3.2 | MIT OR Apache-2.0 | https://github.com/andrewhickman/fs-err |
| fs4 | 1.1.0 | MIT OR Apache-2.0 | https://github.com/al8n/fs4 |
| fs_extra | 1.3.0 | MIT | https://github.com/webdesus/fs_extra |
| fst | 0.4.7 | Unlicense/MIT | https://github.com/BurntSushi/fst |
| funty | 2.0.0 | MIT | https://github.com/myrrlyn/funty |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-lite | 2.6.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/futures-lite |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| generic-array | 0.14.7 | MIT | https://github.com/fizyk20/generic-array.git |
| geo | 0.33.1 | MIT OR Apache-2.0 | https://github.com/georust/geo |
| geo-types | 0.7.20 | MIT OR Apache-2.0 | https://github.com/georust/geo |
| geographiclib-rs | 0.2.7 | MIT | https://github.com/georust/geographiclib-rs |
| geohash | 0.13.2 | MIT/Apache-2.0 | https://github.com/georust/geohash.rs |
| getrandom | 0.1.16 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| gimli | 0.34.0 | MIT OR Apache-2.0 | https://github.com/gimli-rs/gimli |
| half | 1.8.3 | MIT OR Apache-2.0 | https://github.com/starkat99/half-rs |
| half | 2.7.1 | MIT OR Apache-2.0 | https://github.com/VoidStarKat/half-rs |
| hash32 | 0.3.1 | MIT OR Apache-2.0 | https://github.com/japaric/hash32 |
| hashbrown | 0.12.3 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.14.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| heapless | 0.8.0 | MIT OR Apache-2.0 | https://github.com/rust-embedded/heapless |
| heck | 0.5.0 | MIT OR Apache-2.0 | https://github.com/withoutboats/heck |
| hermit-abi | 0.5.3 | MIT OR Apache-2.0 | https://github.com/hermit-os/hermit-rs |
| hex | 0.4.3 | MIT OR Apache-2.0 | https://github.com/KokaKiwi/rust-hex |
| hkdf | 0.12.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/KDFs/ |
| hmac | 0.12.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/MACs |
| http | 1.5.0 | MIT OR Apache-2.0 | https://github.com/hyperium/http |
| http-body | 1.1.0 | MIT | https://github.com/hyperium/http-body |
| http-body-util | 0.1.5 | MIT | https://github.com/hyperium/http-body |
| httparse | 1.10.1 | MIT OR Apache-2.0 | https://github.com/seanmonstar/httparse |
| httpdate | 1.0.3 | MIT OR Apache-2.0 | https://github.com/pyfisch/httpdate |
| humantime | 2.4.0 | MIT OR Apache-2.0 | https://github.com/chronotope/humantime |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hybrid-array |
| hyper | 1.12.0 | MIT | https://github.com/hyperium/hyper |
| hyper-util | 0.1.21 | MIT | https://github.com/hyperium/hyper-util |
| i_float | 1.16.0 | MIT | https://github.com/iShape-Rust/iFloat |
| i_key_sort | 0.10.3 | MIT | https://github.com/iShape-Rust/iKeySort |
| i_overlay | 4.5.2 | MIT OR Apache-2.0 | https://github.com/iShape-Rust/iOverlay |
| i_shape | 1.18.0 | MIT | https://github.com/iShape-Rust/iShape |
| i_tree | 0.18.0 | MIT | https://github.com/iShape-Rust/iTree |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 | https://github.com/strawlab/iana-time-zone |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 | https://github.com/strawlab/iana-time-zone |
| icu_collections | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_locale_core | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer_data | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties_data | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_provider | 2.4.0 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| ident_case | 1.0.1 | MIT/Apache-2.0 | https://github.com/TedDriggs/ident_case |
| idna | 1.1.0 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | https://github.com/hsivonen/idna_adapter |
| indexmap | 1.9.3 | Apache-2.0 OR MIT | https://github.com/bluss/indexmap |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/indexmap |
| inout | 0.1.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| integer-encoding | 4.1.0 | MIT | https://github.com/dermesser/integer-encoding-rs |
| io-uring | 0.7.15 | MIT OR Apache-2.0 | https://github.com/tokio-rs/io-uring |
| irg-kvariants | 0.1.1 | MIT | https://github.com/meilisearch/charabia |
| is_terminal_polyfill | 1.70.2 | MIT OR Apache-2.0 | https://github.com/polyfill-rs/is_terminal_polyfill |
| itertools | 0.15.0 | MIT OR Apache-2.0 | https://github.com/rust-itertools/itertools |
| itoa | 1.0.18 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| jiff | 0.2.39 | Unlicense OR MIT | https://github.com/BurntSushi/jiff |
| jiff-core | 0.1.2 | Unlicense OR MIT | https://github.com/BurntSushi/jiff |
| jiff-static | 0.2.39 | Unlicense OR MIT | https://github.com/BurntSushi/jiff |
| jiff-tzdb | 0.1.9 | Unlicense OR MIT | https://github.com/BurntSushi/jiff |
| jiff-tzdb-platform | 0.1.3 | Unlicense OR MIT | https://github.com/BurntSushi/jiff |
| js-sys | 0.3.107 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys |
| keyring | 3.6.3 | MIT OR Apache-2.0 | https://github.com/hwchen/keyring-rs.git |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 | https://github.com/rust-lang-nursery/lazy-static.rs |
| libc | 0.2.191 | MIT OR Apache-2.0 | https://github.com/rust-lang/libc |
| libm | 0.2.16 | MIT | https://github.com/rust-lang/compiler-builtins |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/sunfishcode/linux-raw-sys |
| linux-raw-sys | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/sunfishcode/linux-raw-sys |
| litemap | 0.8.3 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| log | 0.4.35 | MIT OR Apache-2.0 | https://github.com/rust-lang/log |
| lz4_flex | 0.14.0 | MIT | https://github.com/pseitz/lz4_flex |
| macro_rules_attribute | 0.2.3 | Apache-2.0 OR MIT OR Zlib | https://github.com/danielhenrymantilla/macro_rules_attribute-rs |
| macro_rules_attribute-proc_macro | 0.2.3 | Apache-2.0 OR MIT OR Zlib | https://github.com/danielhenrymantilla/macro_rules_attribute-rs |
| matchit | 0.8.4 | MIT AND BSD-3-Clause | https://github.com/ibraheemdev/matchit |
| memchr | 2.8.3 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| memmap2 | 0.9.12 | MIT OR Apache-2.0 | https://github.com/RazrFalcon/memmap2-rs |
| memoffset | 0.9.1 | MIT | https://github.com/Gilnaa/memoffset |
| mime | 0.3.18 | MIT OR Apache-2.0 | https://github.com/hyperium/mime |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| mio | 1.2.4 | MIT | https://github.com/tokio-rs/mio |
| moka | 0.12.16 | (MIT OR Apache-2.0) AND Apache-2.0 | https://github.com/moka-rs/moka |
| murmur3_32 | 0.1.0 | MIT OR Apache-2.0 | https://github.com/jsen-/murmur3_32 |
| nix | 0.25.1 | MIT | https://github.com/nix-rust/nix |
| nix | 0.29.0 | MIT | https://github.com/nix-rust/nix |
| nix | 0.31.3 | MIT | https://github.com/nix-rust/nix |
| nodrop | 0.1.14 | MIT/Apache-2.0 | https://github.com/bluss/arrayvec |
| nom | 8.0.0 | MIT | https://github.com/rust-bakery/nom |
| ntapi | 0.4.3 | Apache-2.0 OR MIT | https://github.com/MSxDOS/ntapi |
| num | 0.4.3 | MIT OR Apache-2.0 | https://github.com/rust-num/num |
| num-bigint | 0.4.8 | MIT OR Apache-2.0 | https://github.com/rust-num/num-bigint |
| num-cmp | 0.1.0 | MIT/Apache-2.0 | https://github.com/lifthrasiir/num-cmp |
| num-complex | 0.4.6 | MIT OR Apache-2.0 | https://github.com/rust-num/num-complex |
| num-derive | 0.4.2 | MIT OR Apache-2.0 | https://github.com/rust-num/num-derive |
| num-integer | 0.1.47 | MIT OR Apache-2.0 | https://github.com/rust-num/num-integer |
| num-iter | 0.1.46 | MIT OR Apache-2.0 | https://github.com/rust-num/num-iter |
| num-rational | 0.4.2 | MIT OR Apache-2.0 | https://github.com/rust-num/num-rational |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | https://github.com/rust-num/num-traits |
| num_cpus | 1.17.0 | MIT OR Apache-2.0 | https://github.com/seanmonstar/num_cpus |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/madsmtm/objc2 |
| objc2-io-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/madsmtm/objc2 |
| object | 0.40.0 | Apache-2.0 OR MIT | https://github.com/gimli-rs/object |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | https://github.com/matklad/once_cell |
| once_cell_polyfill | 1.70.2 | MIT OR Apache-2.0 | https://github.com/polyfill-rs/once_cell_polyfill |
| opaque-debug | 0.3.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| ordered-float | 2.10.1 | MIT | https://github.com/reem/rust-ordered-float |
| ordered-float | 5.5.1 | MIT | https://github.com/reem/rust-ordered-float |
| ordered-stream | 0.2.0 | MIT OR Apache-2.0 | https://github.com/danieldg/ordered-stream |
| parking | 2.2.1 | Apache-2.0 OR MIT | https://github.com/smol-rs/parking |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| password-hash | 0.5.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits/tree/master/password-hash |
| pastey | 0.2.3 | MIT OR Apache-2.0 | https://github.com/as1100k/pastey |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| permutation_iterator | 0.1.2 | Apache-2.0 | https://github.com/asimihsan/permutation-iterator-rs |
| petgraph | 0.6.5 | MIT OR Apache-2.0 | https://github.com/petgraph/petgraph |
| ph | 0.8.5 | MIT OR Apache-2.0 | https://github.com/beling/bsuccinct-rs |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project-lite |
| piper | 0.2.5 | MIT OR Apache-2.0 | https://github.com/smol-rs/piper |
| png | 0.17.16 | MIT OR Apache-2.0 | https://github.com/image-rs/image-png |
| polling | 3.11.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/polling |
| poly1305 | 0.8.0 | Apache-2.0 OR MIT | https://github.com/RustCrypto/universal-hashes |
| portable-atomic | 1.15.0 | Apache-2.0 OR MIT | https://github.com/taiki-e/portable-atomic |
| portable-atomic-util | 0.2.8 | Apache-2.0 OR MIT | https://github.com/taiki-e/portable-atomic-util |
| potential_utf | 0.1.6 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 | https://github.com/cryptocorrosion/cryptocorrosion |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | https://github.com/bkchr/proc-macro-crate |
| proc-macro-error-attr3 | 3.1.1 | MIT OR Apache-2.0 | https://github.com/gamma0987/proc-macro-error3 |
| proc-macro-error3 | 3.1.1 | MIT OR Apache-2.0 | https://github.com/gamma0987/proc-macro-error3 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| proc-macro2-diagnostics | 0.10.1 | MIT/Apache-2.0 | https://github.com/SergioBenitez/proc-macro2-diagnostics |
| procfs | 0.18.0 | MIT OR Apache-2.0 | https://github.com/eminence/procfs |
| procfs-core | 0.18.0 | MIT OR Apache-2.0 | https://github.com/eminence/procfs |
| qdrant-rust-stemmers | 1.3.0 | MIT/BSD-3-Clause | https://github.com/qdrant/rust-stemmers |
| quick_cache | 0.7.0 | MIT | https://github.com/arthurprs/quick-cache |
| quote | 1.0.47 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | https://github.com/r-efi/r-efi |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | https://github.com/r-efi/r-efi |
| radium | 0.7.0 | MIT | https://github.com/bitvecto-rs/radium |
| rand | 0.10.3 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand | 0.7.3 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand | 0.8.8 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_chacha | 0.2.2 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_chacha | 0.3.1 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_core | 0.10.1 | MIT OR Apache-2.0 | https://github.com/rust-random/rand_core |
| rand_core | 0.5.1 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_core | 0.6.4 | MIT OR Apache-2.0 | https://github.com/rust-random/rand |
| rand_distr | 0.6.0 | MIT OR Apache-2.0 | https://github.com/rust-random/rand_distr |
| rand_hc | 0.2.0 | MIT/Apache-2.0 | https://github.com/rust-random/rand |
| rand_pcg | 0.10.2 | MIT OR Apache-2.0 | https://github.com/rust-random/rngs |
| rayon | 1.12.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon |
| rayon-core | 1.13.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon |
| redb | 3.1.3 | MIT OR Apache-2.0 | https://github.com/cberner/redb |
| redox_syscall | 0.5.18 | MIT | https://gitlab.redox-os.org/redox-os/syscall |
| regex | 1.13.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/regex |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 | https://github.com/rust-lang/regex |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | https://github.com/rust-lang/regex |
| rmp | 0.8.15 | MIT | https://github.com/3Hren/msgpack-rust |
| rmp-serde | 1.3.1 | MIT | https://github.com/3Hren/msgpack-rust |
| roaring | 0.11.5 | MIT OR Apache-2.0 | https://github.com/RoaringBitmap/roaring-rs |
| robust | 1.2.0 | MIT OR Apache-2.0 | https://github.com/georust/robust |
| rstar | 0.12.2 | MIT OR Apache-2.0 | https://github.com/georust/rstar |
| rustc-demangle | 0.1.28 | MIT/Apache-2.0 | https://github.com/rust-lang/rustc-demangle |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | https://github.com/djc/rustc-version-rs |
| rustix | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/rustix |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/rustix |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | https://github.com/dtolnay/rustversion |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 | https://github.com/dtolnay/ryu |
| same-file | 1.0.6 | Unlicense/MIT | https://github.com/BurntSushi/same-file |
| schemars | 0.8.22 | MIT | https://github.com/GREsau/schemars |
| schemars_derive | 0.8.22 | MIT | https://github.com/GREsau/schemars |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | https://github.com/bluss/scopeguard |
| secret-service | 4.0.0 | MIT OR Apache-2.0 | https://github.com/hwchen/secret-service-rs.git |
| security-framework | 2.11.1 | MIT OR Apache-2.0 | https://github.com/kornelski/rust-security-framework |
| security-framework | 3.7.0 | MIT OR Apache-2.0 | https://github.com/kornelski/rust-security-framework |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 | https://github.com/kornelski/rust-security-framework |
| seedable_hash | 0.1.7 | MIT OR Apache-2.0 | https://github.com/beling/bsuccinct-rs |
| self_cell | 1.3.0 | Apache-2.0 OR GPL-2.0-only | https://github.com/Voultapher/self_cell |
| semver | 1.0.28 | MIT OR Apache-2.0 | https://github.com/dtolnay/semver |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde-untagged | 0.1.9 | MIT OR Apache-2.0 | https://github.com/dtolnay/serde-untagged |
| serde-value | 0.7.0 | MIT | https://github.com/arcnmx/serde-value |
| serde_cbor | 0.11.2 | MIT/Apache-2.0 | https://github.com/pyfisch/cbor |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive_internals | 0.29.1 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| serde_path_to_error | 0.1.20 | MIT OR Apache-2.0 | https://github.com/dtolnay/path-to-error |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/serde-repr |
| serde_spanned | 1.1.2 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| serde_variant | 0.1.3 | Apache-2.0 OR MIT | https://github.com/d-e-s-o/serde_variant.git |
| sha1 | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha1_smol | 1.0.1 | BSD-3-Clause | https://github.com/mitsuhiko/sha1-smol |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| shlex | 2.0.1 | MIT OR Apache-2.0 | https://github.com/comex/rust-shlex |
| sif-itree | 0.4.1 | MIT OR Apache-2.0 | https://codeberg.org/adamreichold/sif-itree |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 | https://github.com/vorner/signal-hook |
| signature | 2.2.0 | Apache-2.0 OR MIT | https://github.com/RustCrypto/traits/tree/master/signature |
| simd-adler32 | 0.3.10 | MIT | https://github.com/mcountryman/simd-adler32 |
| siphasher | 1.0.4 | MIT OR Apache-2.0 | https://github.com/jedisct1/rust-siphash |
| slab | 0.4.12 | MIT | https://github.com/tokio-rs/slab |
| slice-group-by | 0.3.1 | MIT | https://github.com/Kerollmops/slice-group-by |
| smallvec | 1.16.4 | MIT OR Apache-2.0 | https://github.com/servo/rust-smallvec |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/socket2 |
| spade | 2.15.1 | MIT OR Apache-2.0 | https://github.com/Stoeoef/spade |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | https://github.com/storyyeller/stable_deref_trait |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | https://github.com/nvzqz/static-assertions-rs |
| strsim | 0.10.0 | MIT | https://github.com/dguo/strsim-rs |
| strsim | 0.11.1 | MIT | https://github.com/rapidfuzz/strsim-rs |
| strum | 0.28.0 | MIT | https://github.com/Peternator7/strum |
| strum_macros | 0.28.0 | MIT | https://github.com/Peternator7/strum |
| subtle | 2.6.1 | BSD-3-Clause | https://github.com/dalek-cryptography/subtle |
| syn | 2.0.119 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 3.0.9 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| sync_wrapper | 1.0.2 | Apache-2.0 | https://github.com/Actyx/sync_wrapper |
| synstructure | 0.14.0 | MIT | https://github.com/mystor/synstructure |
| sysinfo | 0.39.6 | MIT | https://github.com/GuillaumeGomez/sysinfo |
| tagptr | 0.2.0 | MIT/Apache-2.0 | https://github.com/oliver-giersch/tagptr.git |
| tap | 1.0.1 | MIT | https://github.com/myrrlyn/tap |
| tar | 0.4.46 | MIT OR Apache-2.0 | https://github.com/composefs/tar-rs |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | https://github.com/Stebalien/tempfile |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thread-priority | 3.1.1 | MIT | https://github.com/iddm/thread-priority |
| tinystr | 0.8.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| tinyvec | 1.13.3 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/tinyvec |
| tokio | 1.54.1 | MIT | https://github.com/tokio-rs/tokio |
| tokio-macros | 2.7.2 | MIT | https://github.com/tokio-rs/tokio |
| toml | 0.9.12+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_datetime | 0.7.5+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_edit | 0.25.18+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| toml_writer | 1.2.0+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml |
| tower | 0.5.3 | MIT | https://github.com/tower-rs/tower |
| tower-layer | 0.3.3 | MIT | https://github.com/tower-rs/tower |
| tower-service | 0.3.3 | MIT | https://github.com/tower-rs/tower |
| tracing | 0.1.44 | MIT | https://github.com/tokio-rs/tracing |
| tracing-attributes | 0.1.31 | MIT | https://github.com/tokio-rs/tracing |
| tracing-core | 0.1.36 | MIT | https://github.com/tokio-rs/tracing |
| typeid | 1.0.3 | MIT OR Apache-2.0 | https://github.com/dtolnay/typeid |
| typenum | 1.20.1 | MIT OR Apache-2.0 | https://github.com/paholg/typenum |
| uds_windows | 1.2.1 | MIT | https://github.com/haraldh/rust_uds_windows |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| unicode-normalization | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/unicode-rs/unicode-normalization |
| universal-hash | 0.5.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| unty | 0.0.4 | MIT OR Apache-2.0 | https://github.com/bincode-org/unty |
| ureq | 3.5.1 | MIT OR Apache-2.0 | https://github.com/algesten/ureq |
| ureq-proto | 0.7.1 | MIT OR Apache-2.0 | https://github.com/algesten/ureq-proto |
| url | 2.5.8 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| utf8-zero | 0.8.1 | MIT OR Apache-2.0 | https://github.com/algesten/utf8-zero |
| utf8_iter | 1.1.0 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf8_iter |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT | https://github.com/alacritty/vte |
| utf_types | 1.0.0 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf_types |
| uuid | 1.29.0 | Apache-2.0 OR MIT | https://github.com/uuid-rs/uuid |
| validator | 0.20.0 | MIT | https://github.com/Keats/validator |
| validator_derive | 0.20.1 | MIT | https://github.com/Keats/validator |
| vaporetto | 0.6.5 | MIT OR Apache-2.0 | https://github.com/daac-tools/vaporetto |
| version_check | 0.9.5 | MIT/Apache-2.0 | https://github.com/SergioBenitez/version_check |
| virtue | 0.0.18 | MIT | https://github.com/bincode-org/virtue |
| walkdir | 2.5.0 | Unlicense/MIT | https://github.com/BurntSushi/walkdir |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi |
| wasi | 0.9.0+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wasi-rs |
| wasm-bindgen | 0.2.130 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen |
| wasm-bindgen-macro | 0.2.130 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro |
| wasm-bindgen-macro-support | 0.2.130 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support |
| wasm-bindgen-shared | 0.2.130 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared |
| whatlang | 0.16.4 | MIT | https://github.com/greyblake/whatlang-rs |
| winapi | 0.3.9 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-i686-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-util | 0.1.11 | Unlicense OR MIT | https://github.com/BurntSushi/winapi-util |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| windows | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-future | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-targets | 0.53.5 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_aarch64_gnullvm | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_aarch64_msvc | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnu | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_gnullvm | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_i686_msvc | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnu | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnullvm | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_msvc | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| winnow | 0.7.15 | MIT | https://github.com/winnow-rs/winnow |
| winnow | 1.0.5 | MIT | https://github.com/winnow-rs/winnow |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | https://github.com/bytecodealliance/wit-bindgen |
| writeable | 0.6.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| wyhash | 0.5.0 | MIT OR Apache-2.0 | https://github.com/eldruin/wyhash-rs |
| wyz | 0.5.1 | MIT | https://github.com/myrrlyn/wyz |
| xattr | 1.7.0 | MIT OR Apache-2.0 | https://github.com/Stebalien/xattr |
| xdg-home | 1.3.0 | MIT | https://github.com/zeenix/xdg-home |
| xxhash-rust | 0.8.19 | BSL-1.0 | https://github.com/DoumanAsh/xxhash-rust |
| yoke | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| yoke-derive | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zbus | 4.4.0 | MIT | https://github.com/dbus2/zbus/ |
| zbus | 5.19.0 | MIT | https://github.com/z-galaxy/zbus/ |
| zbus_macros | 4.4.0 | MIT | https://github.com/dbus2/zbus/ |
| zbus_macros | 5.19.0 | MIT | https://github.com/z-galaxy/zbus/ |
| zbus_names | 3.0.0 | MIT | https://github.com/dbus2/zbus/ |
| zbus_names | 4.3.5 | MIT | https://github.com/z-galaxy/zbus/ |
| zcheapstr | 1.2.0 | MIT | https://github.com/z-galaxy/zcheapstr/ |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | https://github.com/google/zerocopy |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | https://github.com/google/zerocopy |
| zerofrom | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zeroize | 1.9.1 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| zerotrie | 0.2.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec | 0.11.9 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec-derive | 0.11.7 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zmij | 1.0.23 | MIT | https://github.com/dtolnay/zmij |
| zvariant | 4.2.0 | MIT | https://github.com/dbus2/zbus/ |
| zvariant | 5.15.1 | MIT | https://github.com/z-galaxy/zbus/ |
| zvariant_derive | 4.2.0 | MIT | https://github.com/dbus2/zbus/ |
| zvariant_derive | 5.15.1 | MIT | https://github.com/z-galaxy/zbus/ |
| zvariant_utils | 2.1.0 | MIT | https://github.com/dbus2/zbus/ |
| zvariant_utils | 4.2.0 | MIT | https://github.com/z-galaxy/zbus/ |

## 6. Rust dev/test-only crates (not in shipped binaries)

| Crate | Version | Licence (declared) | Repository |
|---|---|---|---|
| alloca | 0.4.0 | MIT | https://github.com/playXE/alloca-rs |
| anes | 0.1.6 | MIT OR Apache-2.0 | https://github.com/zrzka/anes-rs |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | https://github.com/dtolnay/anyhow |
| ar_archive_writer | 0.5.3 | Apache-2.0 WITH LLVM-exception | https://github.com/rust-lang/ar_archive_writer |
| aws-lc-rs | 1.19.0 | ISC AND (Apache-2.0 OR ISC) | https://github.com/aws/aws-lc-rs |
| aws-lc-sys | 0.47.0 | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) | https://github.com/aws/aws-lc-rs |
| bustle | 0.5.1 | Apache-2.0 | https://github.com/jonhoo/bustle.git |
| cast | 0.3.0 | MIT OR Apache-2.0 | https://github.com/japaric/cast.rs |
| ciborium | 0.2.2 | Apache-2.0 | https://github.com/enarx/ciborium |
| ciborium-io | 0.2.2 | Apache-2.0 | https://github.com/enarx/ciborium |
| ciborium-ll | 0.2.2 | Apache-2.0 | https://github.com/enarx/ciborium |
| clap | 4.6.7 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| clap_builder | 4.6.7 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| clap_derive | 4.6.7 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| clap_lex | 1.1.1 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap |
| cmake | 0.1.58 | MIT OR Apache-2.0 | https://github.com/rust-lang/cmake-rs |
| colorz | 1.1.4 | MIT OR Apache-2.0 | https://github.com/RustyYato/colorz |
| combine | 4.6.9 | MIT | https://github.com/Marwes/combine |
| console | 0.16.6 | MIT | https://github.com/console-rs/console |
| core_detect | 1.0.0 | MIT/Apache-2.0 | https://github.com/thomcc/core_detect |
| cpp_demangle | 0.4.5 | MIT OR Apache-2.0 | https://github.com/gimli-rs/cpp_demangle |
| crc | 3.4.0 | MIT OR Apache-2.0 | https://github.com/mrhooray/crc-rs.git |
| crc-catalog | 2.5.0 | MIT OR Apache-2.0 | https://github.com/akhilles/crc-catalog.git |
| crc-fast | 1.10.0 | MIT OR Apache-2.0 | https://github.com/awesomized/crc-fast-rust |
| criterion | 0.8.2 | Apache-2.0 OR MIT | https://github.com/criterion-rs/criterion.rs |
| criterion-plot | 0.8.2 | Apache-2.0 OR MIT | https://github.com/criterion-rs/criterion.rs |
| debugid | 0.8.0 | Apache-2.0 | https://github.com/getsentry/rust-debugid |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 | https://gitlab.com/kornelski/dunce |
| encode_unicode | 1.0.0 | Apache-2.0 OR MIT | https://github.com/tormol/encode_unicode |
| faccess | 0.2.4 | MIT | https://github.com/Freaky/faccess |
| findshlibs | 0.10.2 | MIT OR Apache-2.0 | https://github.com/gimli-rs/findshlibs |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs |
| glob | 0.3.4 | MIT OR Apache-2.0 | https://github.com/rust-lang/glob |
| glob-match | 0.2.1 | MIT |  |
| goblin | 0.7.1 | MIT | https://github.com/m4b/goblin |
| h2 | 0.4.20 | MIT | https://github.com/hyperium/h2 |
| hdrhistogram | 7.6.0 | MIT/Apache-2.0 | https://github.com/HdrHistogram/HdrHistogram_rust.git |
| hyper-rustls | 0.27.10 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/hyper-rustls |
| indicatif | 0.18.6 | MIT | https://github.com/console-rs/indicatif |
| ipnet | 2.12.2 | MIT OR Apache-2.0 | https://github.com/krisprice/ipnet |
| is-terminal | 0.4.17 | MIT | https://github.com/sunfishcode/is-terminal |
| is_ci | 1.2.0 | ISC | https://github.com/zkat/is_ci |
| itertools | 0.12.1 | MIT OR Apache-2.0 | https://github.com/rust-itertools/itertools |
| itertools | 0.13.0 | MIT OR Apache-2.0 | https://github.com/rust-itertools/itertools |
| jni | 0.22.4 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-rs |
| jni-macros | 0.22.4 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-rs |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-sys |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 | https://github.com/jni-rs/jni-sys |
| jobserver | 0.1.35 | MIT OR Apache-2.0 | https://github.com/rust-lang/jobserver-rs |
| libloading | 0.8.9 | ISC | https://github.com/nagisa/rust_libloading/ |
| matrixmultiply | 0.3.11 | MIT/Apache-2.0 | https://github.com/bluss/matrixmultiply/ |
| md-5 | 0.11.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| multimap | 0.10.1 | MIT OR Apache-2.0 | https://github.com/havarnov/multimap |
| ndarray | 0.17.2 | MIT OR Apache-2.0 | https://github.com/rust-ndarray/ndarray |
| ndarray-npy | 0.10.0 | MIT OR Apache-2.0 | https://github.com/jturner314/ndarray-npy |
| nix | 0.26.4 | MIT | https://github.com/nix-rust/nix |
| num-format | 0.4.4 | MIT/Apache-2.0 | https://github.com/bcmyers/num-format |
| num_threads | 0.1.8 | MIT OR Apache-2.0 | https://github.com/jhpratt/num_threads |
| object | 0.39.1 | Apache-2.0 OR MIT | https://github.com/gimli-rs/object |
| object_store | 0.14.2 | MIT/Apache-2.0 | https://github.com/apache/arrow-rs-object-store |
| oorandom | 11.1.5 | MIT | https://hg.sr.ht/~icefox/oorandom |
| openssl-probe | 0.2.2 | MIT OR Apache-2.0 | https://github.com/rustls/openssl-probe |
| page_size | 0.6.0 | MIT/Apache-2.0 | https://github.com/Elzair/page_size_rs |
| path_facts | 0.2.2 | MIT/Apache-2.0 | https://github.com/schneems/path_facts |
| pest | 2.9.5 | MIT OR Apache-2.0 | https://github.com/pest-parser/pest |
| pest_derive | 2.9.5 | MIT OR Apache-2.0 | https://github.com/pest-parser/pest |
| pest_generator | 2.9.5 | MIT OR Apache-2.0 | https://github.com/pest-parser/pest |
| pest_meta | 2.9.5 | MIT OR Apache-2.0 | https://github.com/pest-parser/pest |
| pest_stack_guard | 1.0.1 | MIT OR Apache-2.0 | https://github.com/pest-parser/pest_stack_guard |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/pkg-config-rs |
| plain | 0.2.3 | MIT/Apache-2.0 | https://github.com/randomites/plain |
| plotters | 0.3.7 | MIT | https://github.com/plotters-rs/plotters |
| plotters-backend | 0.3.7 | MIT | https://github.com/plotters-rs/plotters |
| plotters-svg | 0.3.7 | MIT | https://github.com/plotters-rs/plotters.git |
| pprof | 0.15.0 | Apache-2.0 | https://github.com/tikv/pprof-rs |
| prettyplease | 0.2.37 | MIT OR Apache-2.0 | https://github.com/dtolnay/prettyplease |
| proptest | 1.12.0 | MIT OR Apache-2.0 | https://github.com/proptest-rs/proptest |
| prost | 0.12.6 | Apache-2.0 | https://github.com/tokio-rs/prost |
| prost-build | 0.12.6 | Apache-2.0 | https://github.com/tokio-rs/prost |
| prost-derive | 0.12.6 | Apache-2.0 | https://github.com/tokio-rs/prost |
| prost-types | 0.12.6 | Apache-2.0 | https://github.com/tokio-rs/prost |
| psm | 0.1.32 | MIT OR Apache-2.0 | https://github.com/rust-lang/stacker/ |
| py_literal | 0.4.0 | MIT OR Apache-2.0 | https://github.com/jturner314/py_literal |
| quick-xml | 0.26.0 | MIT | https://github.com/tafia/quick-xml |
| quick-xml | 0.41.0 | MIT | https://github.com/tafia/quick-xml |
| quickcheck | 1.1.0 | Unlicense OR MIT | https://github.com/BurntSushi/quickcheck |
| rand_xorshift | 0.5.0 | MIT OR Apache-2.0 | https://github.com/rust-random/rngs |
| rawpointer | 0.2.1 | MIT/Apache-2.0 | https://github.com/bluss/rawpointer/ |
| relative-path | 1.9.3 | MIT OR Apache-2.0 | https://github.com/udoprog/relative-path |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest |
| rgb | 0.8.53 | MIT | https://github.com/kornelski/rust-rgb |
| rstest | 0.26.1 | MIT OR Apache-2.0 | https://github.com/la10736/rstest |
| rstest_macros | 0.26.1 | MIT OR Apache-2.0 | https://github.com/la10736/rstest |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls |
| rustls-native-certs | 0.8.5 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls-native-certs |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 | https://github.com/rustls/pki-types |
| rustls-platform-verifier | 0.7.1 | MIT OR Apache-2.0 | https://github.com/rustls/rustls-platform-verifier |
| rustls-platform-verifier-android | 0.2.0 | MIT OR Apache-2.0 | https://github.com/rustls/rustls-platform-verifier |
| rustls-webpki | 0.103.16 | ISC | https://github.com/rustls/webpki |
| schannel | 0.1.30 | MIT | https://github.com/steffengy/schannel-rs |
| scroll | 0.11.0 | MIT | https://github.com/m4b/scroll |
| scroll_derive | 0.11.1 | MIT | https://github.com/m4b/scroll |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 | https://github.com/nox/serde_urlencoded |
| simd_cesu8 | 1.2.0 | Apache-2.0 OR MIT | https://github.com/seancroach/simd_cesu8 |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 | https://github.com/rusticstuff/simdutf8 |
| spin | 0.10.1 | MIT | https://github.com/mvdnes/spin-rs.git |
| stacker | 0.1.25 | MIT OR Apache-2.0 | https://github.com/rust-lang/stacker |
| str_stack | 0.1.1 | MIT/Apache-2.0 | https://github.com/Stebalien/str_stack |
| supports-color | 3.0.2 | Apache-2.0 | https://github.com/zkat/supports-color |
| symbolic-common | 12.18.3 | MIT | https://github.com/getsentry/symbolic |
| symbolic-demangle | 12.18.3 | MIT | https://github.com/getsentry/symbolic |
| tango-bench | 0.7.2 | MIT | https://github.com/bazhenov/tango |
| tinytemplate | 1.2.1 | Apache-2.0 OR MIT | https://github.com/bheisler/TinyTemplate |
| tokio-rustls | 0.26.6 | MIT OR Apache-2.0 | https://github.com/rustls/tokio-rustls |
| tokio-util | 0.7.20 | MIT | https://github.com/tokio-rs/tokio |
| tower-http | 0.6.11 | MIT | https://github.com/tower-rs/tower-http |
| try-lock | 0.2.5 | MIT | https://github.com/seanmonstar/try-lock |
| ucd-trie | 0.1.7 | MIT OR Apache-2.0 | https://github.com/BurntSushi/ucd-generate |
| unarray | 0.1.4 | MIT OR Apache-2.0 | https://github.com/cameron1024/unarray |
| unicode-width | 0.2.3 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-width |
| unit-prefix | 0.5.2 | MIT | https://codeberg.org/commons-rs/unit-prefix |
| untrusted | 0.9.0 | ISC | https://github.com/briansmith/untrusted |
| want | 0.3.2 | MIT | https://github.com/seanmonstar/want |
| wasm-bindgen-futures | 0.4.80 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures |
| wasm-streams | 0.5.0 | MIT OR Apache-2.0 | https://github.com/MattiasBuelens/wasm-streams/ |
| web-sys | 0.3.107 | MIT OR Apache-2.0 | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys |
| web-time | 1.1.0 | MIT OR Apache-2.0 | https://github.com/daxpedda/web-time |
| webpki-root-certs | 1.0.11 | CDLA-Permissive-2.0 | https://github.com/rustls/webpki-roots |
