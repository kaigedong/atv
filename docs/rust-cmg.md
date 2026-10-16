# Native CMG media transform

`native/crates/atv-cmg` replaces the AVC CMG Live modes 1/2 media
transform with readable Rust. Its production code has no JavaScript engine,
WebView, WASM engine, official binary, embedded virtual-machine program, or
network dependency. It returns compressed AVC NAL units for the platform video
decoder; it does not decode H.264 into pixels.

The supported scope is explicit: AVC, CMG Live modes 1 and 2. Mode 1 has real
stream and component differential evidence; mode 2 has component differential
evidence but no separately identified mode 2 broadcast capture. Clear AVC passes
through. VOD returns an error rather than emitting protected slices as clear
video. HEVC/AVS and audio-specific vendor transform paths have not been
reconstructed. Android playback acceptance is recorded separately.

## Artifact and call-boundary evidence

The selected local official `cmg.worker.js` embeds a 64,644-byte WASM in a data
URI. REA's WABT 1.0.42 adapter validated it and retained evidence
`ev_a9265bffaafd165171559dc3b07392dfd8d43df464c583003453203126a2a5c2`.

| Artifact | SHA-256 |
| --- | --- |
| `cmg.worker.js` | `c719aa90eba78a0bc9d7be29ccffce6a851c3f968770cacc883173230577f62a` |
| `hls.cmg.js` | `70394b440f6c57127501d07a2e48bb554c348104fa9a3331272aa6ed91c3b5de` |
| Embedded CMG WASM | `dba01d8debba97420dd1a498059459bde7107a2aaf4bb003ae7e04cf39c1ed35` |

Static inspection found 440 internal WASM functions. Function 51 is a custom
instruction interpreter; the worker's JavaScript initialises a separate
378,592-byte program/data region. The exported `_CMG_jsdecLive8` wrapper enters
that interpreter at program offset 232408. Controlled tracing of an authorised
real IDR NAL followed its sparse-block helper to program 118672. That helper
executes the TEA arithmetic below, rather than a video codec.

`hls.cmg.js` calls the vendor media module after AVC NAL extraction and before
remuxing. SPS NAL byte 2's low two bits equal to 1 or 2 enable Live processing.
The JavaScript clears SPS byte 2 after processing. Normal H.264 picture decoding
continues through the browser media pipeline. Its separate ticket WASM belongs
to authorisation, not this media transform.

During legitimate baseline playback, temporary wrappers preserved the original
function arguments, return values and memory behaviour while recording 20 NAL
calls. The wrappers were restored afterwards. Ten protected picture NAL units
provided full input/output pairs: `_CMG_jsdecLive8` changed sparse eight-byte
blocks; the SPS calls returned unchanged bytes. A saved CMG memory snapshot
replayed the selected call in the original component byte-for-byte offline.
These development references remain in ignored `research/native-cmg/`; none
is loaded by the product.

## Implemented algorithm

One `CmgSession` belongs to one stream. The caller supplies complete AVC NALs
without a start code or AVCC length prefix, in stream order. Create a fresh
session when changing channel or resetting the demux/codec.

For a mode 1 or 2 SPS, the Rust wrapper clears byte 2, matching the official
JavaScript glue. For protected type 1 and type 5 slices:

1. If the original NAL length is at most 128 bytes, return it unchanged.
2. Remove `00 00 03` only when followed by a byte in `00..03`.
3. Read a 128-bit key as four little-endian words from NAL offset 5 for type 5,
   or offset 1 for type 1. Those key bytes are part of the normal received NAL;
   no account token or private server key is embedded in the implementation.
4. Set the sparse interval to `160 + 32 * (first_key_byte & 3)`.
5. Starting at NAL offset 32, decrypt each eight-byte block while
   `offset < unescaped_length - 16`, advancing by the sparse interval.
6. Each block uses 16 TEA rounds, unsigned 32-bit wrapping arithmetic,
   little-endian words, delta `0x9e3779b9`, and initial sum `0xe3779b90`.

The final 16-byte margin and pre-unescape 128-byte threshold are significant.
Treating every complete eight-byte tail block as encrypted caused real decoder
errors; interpreting all IDR intervals as 256 was also an over-generalisation.
The final implementation is checked against the broader synthetic oracle set.

`transform_nal` reproduces the original media component's NAL output, including
its removal of emulation prevention. `transform_annex_b_nal` canonicalises the
result for start-code transport. The latter first removes valid prevention
sequences restored by decryption, then inserts required prevention bytes. A
plain second escaping pass would double-escape restored `00 00 03 01` and
change the decoded picture. The Android media processor uses the Annex B API.

The module performs media processing after the normal resolver grants a stream.
It does not alter server login, preview, membership, region, copyright, platform
or other permission decisions. The historical browser URL is retained as session
context but is not needed by the recovered per-NAL arithmetic. Debug formatting
excludes that URL.

## Validation

`cargo test --manifest-path native/Cargo.toml -p atv-cmg` passes seven tests;
crate Clippy also passes with `--all-targets -- -D warnings`.
The committed `tests/fixtures/live-mode1.tsv` and `live-mode2.tsv` each contain
77 completely synthetic NAL input/output pairs produced by the original
component after the corresponding SPS signal, with no broadcast
frames, service credentials or signed URLs. Coverage includes all four sparse
intervals for both IDR and non-IDR, 128/129-byte thresholds, final-block margins,
emulation-prevention followers `00..03` versus `04`/`ff`, and prevention bytes
restored by decryption. Fixture SHA-256:
`e21357f77b7d2038ecc854d0355dc311b1f9a4eaff5cd289ac6c099da7968097` (mode 1),
`67f746a3ee851e9f0207412b98453e2c9b94769c64eaa2975d3fee9e0bfe92a8` (mode 2).

Two authorised five-second TS captures were extracted into AVC and processed
with the Rust offline example. Each output decoded as 125 frames at 960x540
with FFmpeg's error log empty. The first capture's 627 complete NAL units also
matched the original component's independent offline replay byte-for-byte,
after the same SPS normalisation and four-byte start-code packaging.

| Local validation artifact | Bytes / frames | SHA-256 |
| --- | --- | --- |
| First source TS | 785088 bytes | `5f8ced755e18b39cf512f3a63caf8a3b7e9b6aa82f9f1813a6064a7b7aa241e7` |
| First Rust AVC / original oracle AVC | 673987 bytes / 125 frames | `52c30e260b231c5435a37d11a9acdc6104dd9083b6f51e21e487af2ab91b2967` |
| Second source TS | 585620 bytes | `9eb0525cbdc8962b8b63a02ed6c0d6be96b72bc2fd99720153f78f9f15f059db` |
| Second Rust AVC | 478380 bytes / 125 frames | `5209dee77c9c38a534e551434e0fbdbe125f3bdaa3fe1ed95b4e92325edbd13a` |

The offline tool can be rerun with any legitimately obtained matching stream:

```sh
ffmpeg -v error -i authorised.ts -map 0:v:0 -c copy -f h264 protected.h264
cargo run --manifest-path native/Cargo.toml -p atv-cmg \
  --example transform-annex-b -- protected.h264 clear.h264
ffmpeg -v error -i clear.h264 -f null -
```

The original component was used only as a development reference. Its successful
short-stream differential checks do not prove a ten-minute Android run, audible
output, every channel, a real mode 2 broadcast, VOD, or an alternate upstream
version. Those are
separate verification scopes; this report does not label them complete.

## Additional mode 2 investigation

The selected AVC JavaScript class `fI` uses the same `isLiveDecode` boolean for
SPS signalling values 1 and 2. Its only marker-dependent branch assigns
`isLiveDecode = (sps[2] & 3) == 1 || (sps[2] & 3) == 2`. All subsequent type
1/5 slice calls use that boolean and the same `LIVEVIDEO` argument; no other
AVC branch selects a mode-specific transform. A bounded search of every
`isLiveDecode` property reference in the selected bundle found its AVC
constructor, this SPS assignment, the shared slice gate, and an unrelated HEVC
constructor.

In controlled offline replay, a legitimate captured
SPS was submitted with each of the four possible low-bit values, followed by the
same legitimate IDR NAL. All four IDR outputs were identical. Comparing the
1,632-byte media context after the SPS calls found differences only in its
normal timestamp field; the SPS marker did not select another media algorithm
in that replay.

Each of the 77 synthetic NAL cases was then replayed independently after an
SPS carrying mode 1, and again after mode 2, using the saved normal initialised
media context. All 154 component outputs matched their respective Rust output
byte-for-byte; corresponding mode 1/2 outputs were also identical. The native
implementation therefore handles both documented Live signals through the
same recovered media function. This verifies component behaviour and its glue
contract. The real broadcast captures remain mode 1, so mode 2 channel runtime
acceptance is still unverified and is not inferred from these synthetic cases.
