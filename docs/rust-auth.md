# Native live authorization

`native/crates/atv-auth` replaces the live resolver's request hashing, cKey,
OpenAPI WASM, ticket WASM, normal HTTP token acquisition, and response parsing
with Rust source. Its Android runtime has no JavaScript, JS engine, official
WASM, or WebView dependency. The `research/oracles/generate-auth-oracle.cjs` script is a research
oracle only; it is never linked or shipped in the native application.

## Interfaces and lifetime

```rust
let resolver = atv_auth::LiveResolver::new(atv_auth::ResolverConfig::default())?;
let info = resolver.resolve_live("600001859", "2024078201")?;
let media_url = info.url(); // do not log or persist this expiring signed URL
```

Android must call `generate_device_guid()` once, persist that value in private
application storage, and pass `ResolverConfig { device_guid: Some(saved_guid),
..Default::default() }`. Reuse one shared resolver across channel switches and
pause/resume. `None` generates a normal anonymous GUID for one resolver lifetime,
which is convenient for an isolated research probe; production must not rotate
GUIDs to reset device-bound preview limits. GUID input permits only 1–128 ASCII
letters, digits, underscores and hyphens and remains redacted from Debug.

The resolver is blocking. Run it on a dedicated network thread, never on the
UI, codec or JNI caller thread. Each HTTP operation has a 12-second timeout;
a complete uncached resolution consists of three sequential requests and can
take about 36 seconds under timeout conditions. A cached SDK token removes one
request. Resolver state is serialized with a mutex to keep request sequences
and token-cache state coherent.

`resolve_quality_cancellable(pid, stream_id, quality, adjust, &AtomicBool)` checks
cancellation before locking, after obtaining the mutex, between HTTP operations,
and before returning. The caller may own an `Arc<AtomicBool>` and pass
`cancel.as_ref()`. An in-progress blocking request finishes or times out before
observing cancellation, bounded by 12 seconds. A cancelled call queued behind
an existing call still waits for the existing mutex owner; it makes no new
request when it obtains the lock. The player must also use a generation ID and
discard stale results after channel changes. Dropping a Java-side handle alone
does not cancel a running Rust request.

`LiveInfo` retains the server-selected definition, CDN, preview seconds,
total preview seconds, remaining and total attempts, playback time, formats,
and extra information. The player must enforce preview expiry and any paid
content expiry in extra information; URL resolution alone is not playback
permission for unlimited time. `LiveInfo` and `LiveResolver` Debug output omit
URLs and credentials. Errors contain stage/status/code only, not raw responses
or signed request URLs.

## Actual protocol

1. `POST https://player-api.yangshipin.cn/v1/player/auth` sends the normal
   form parameters `pid`, `guid`, `appid=ysp_pc`, `rand_str`, and the unchanged
   sorted-parameter MD5 signature, obtaining the normal player token and
   server timestamp.
2. If necessary, `GET https://h5access.yangshipin.cn/web/open/token` uses the
   same GUID, public OpenAPI application metadata, timestamp and native
   `token_rnd` to obtain and cache the SDK token. The cache follows the official
   expiry floor of 60 seconds and refreshes on demand after expiry.
3. `POST https://player-api.yangshipin.cn/v1/player/get_live_info` sends the
   actual channel PID/stream ID, requested quality, original fixed protocol
   fields, native cKey, sorted MD5 signature, SDK input/signature, request
   sequence, player token and ticket.

The native user agent identifies itself as `ATV-Native/0.1 (Android TV; Rust)`;
it is also the user-agent input to cKey. Browser metadata fields are explicit
configuration inputs, not a hidden browser runtime. Ordinary initCookie fields
are supplied without creating account, VIP, location or entitlement state.
Normally obtained account cookies can optionally be supplied by the user;
none are embedded in the repository. No response rejection is converted into a
successful stream.

HTTPS is required; redirects are disabled so cookie/token headers cannot
follow redirects to a different endpoint. JSON responses are bounded to
1 MiB. Channel identifiers are validated before requests. Signed query bytes
are retained when appending `revoi` and `extended_param`: decoding/re-encoding
existing `%2f`, `%2B`, etc. would potentially invalidate their signature.

## Reverse-engineering evidence

The pinned baseline resolver comes from vendor SHA-256
`c98fead9682d76f06ffcbbb14546a4c83af8996bde01b5caa30af9001c4864f4`.
Relevant observed functions are `nu/ru` (resolver), `sc/ts` (cKey), `su/au`
(parameter signatures), `Gc` (SDK request MD5), and `_c` (ticket wrapper).

- `cKey`: plaintext has a signed wrapping hash31 over UTF-16 code units,
  the stream ID/time/version/GUID/platform and explicit page metadata. URL,
  lowercased user agent, and referrer keep their first 24 UTF-16 code units.
  AES-128-CBC with PKCS7 produces the `--01` plus uppercase hexadecimal output.
  The implementation is directly compared against the original function,
  including long metadata and empty referrers.
- `keygen_bg.wasm` is a separate 27,669-byte wasm-bindgen artifact loaded by
  `cctvh5-openapicore.min.js`, not by the ticket glue. SHA-256:
  `d977605f3e99879658a332376c55b1dd9e4c89fdc8108623d6252f72a9efed5a`.
  REA evidence ID: `ev_ec6bcd7ea870500a775e1a2768ad0c59296e240c1ba6331dc4e511bf729115f4`.
  WABT decoded its actual imports/exports and Rust compiler metadata. The
  observed `v1` token nonce is MD5 of
  `token_rnd;appid;version;guid;ts;`; signature is MD5 of
  `guid;host;token;protocol;appid;input;`, followed by `-input` in the JS wrapper.
  The SDK's request key sorting uses localeCompare, whereas the player
  signatures use ordinary Array.sort; the Rust implementation preserves this
  distinction for the protocol's fixed ASCII parameter names.
- `RJq7sO71JF.wasm` is a separate 949,810-byte Emscripten/OpenSSL artifact.
  SHA-256: `87c07fafe19e59ceb39a216913a00da5d10b14aa20177beb9d60562efff0cd34`.
  REA evidence ID: `ev_c9cbb1460067df7263fc1d5b811c47de8d9d5e91b0b7ccc89f79fb9c02e2b795`.
  Export `T` / `_aes_encrypt_ctr` calls function 705 to concatenate
  `pid&server_ts&guid&appid&nonce10`. `cnlid` and `appVer`, although JS wrapper
  arguments, do not enter that cipher input. The nonce follows the actual
  shipped musl LCG seeded from client `time(NULL)-1`. Functions 415 and 1218
  derive the fixed AES-128-CTR key/IV from static strings. The Rust implementation
  uses native AES-CTR; no WASM execution remains.

All source assets and complete REA reports remain local under ignored
`research/native-auth/`. They are not included in Android build inputs.
Reacquire assets only from the official URLs already pinned in
历史标签 `baseline/webview-9f20444` 中的 `scripts/prepare-player.mjs`, plus
`https://s.yangshipin.cn/CCTVVideo/cctvh5-openapicore/cctvh5-openapicore.min.js`
and its sibling `keygen_bg.wasm`. Changed hashes require a new review and
fresh differential evidence.

## Verification

Run `cargo test --manifest-path native/Cargo.toml -p atv-auth`.
The original-artifact oracle uses 12 synthetic input sets and checks 84 outputs:
cKey, ticket, token nonce, OpenAPI signature, SDK input hash and both player
parameter signatures. The two real original WASMs are used only while generating
these synthetic fixtures. Reproduce with
`node research/oracles/generate-auth-oracle.cjs` from the repository root
after supplying the pinned research artifacts. No live credentials are present
in the fixtures. Additional tests preserve permission rejection, preview limits,
request cancellation, URL validation and signed query octets.

On 2026-10-16, native HTTP probes successfully resolved CCTV1 HD and FHD,
CCTV2 HD, CCTV13 HD, and Beijing Satellite HD; each returned the requested
quality, preview seconds 0 and a nonempty URL. No live token, signature or URL
was logged or saved. Probe example:

```sh
cargo run --manifest-path native/Cargo.toml -p atv-auth --example probe_live -- 600001811 2029797201
```

This proves live authorization and URL resolution only. Media decode, audio,
continuous ten-minute playback and Android TV interaction are separate gates.

## Known boundary

The observed successful responses have no `chanll` challenge. The old resolver
would Base64-decode that field and evaluate server-supplied JavaScript for
`revoi`. The native implementation deliberately returns
`ResolveError::UnsupportedChallenge` for a nonempty `chanll`; it neither runs
remote code nor pretends to have translated an unobserved challenge. A real
challenge sample and behavior comparison are required before implementing that
additional protocol variant. The OpenAPI rewrite is verified for the normal
production `v1` path; unrelated test/cver modes are not claimed as implemented.
