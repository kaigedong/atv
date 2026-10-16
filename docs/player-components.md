# Official live resolver extraction

> 历史 WebView 基线记录，仅适用于标签 `baseline/webview-9f20444`。其中的网页／Node 构建命令须在该历史版本的独立工作树使用。当前 main 的默认应用为原生版本，见 [main-native-build.md](main-native-build.md)。


Run `node scripts/prepare-player.mjs` to download the pinned official vendor,
verify its SHA-256, and regenerate `web/vendor/live-resolver.vendor.js` plus its
manifest. `--check` compares the generated files without writing. Node.js 20+
is sufficient; there are no parser/build dependencies. Downloads with a changed
hash are rejected before evaluation or output writes.

The script captures the vendor's webpack registration in a restricted Node VM.
It never executes a module factory. Outer module `ed4d` contains a self-contained
live-player package with inner entry `fb15`. Its factory source is retained,
with one export-declaration patch adding `resolveLive: nu` and
`initialize: Hc.initCookie`. The request, authentication,
cKey, signature, and ticket function bodies are unchanged. An outer wrapper
exposes only `window.OfficialLiveResolver.resolveLive` and
`window.OfficialLiveResolver.initialize`. Unused component
definitions remain in the bundle but are never mounted; Vue, Poster, the control
bar, the original site app, recommendations, EPG, Trace, VR, and VConsole are not
part of the mounted product UI.

Call `OfficialLiveResolver.initialize()` once in the genuine official page
context before resolving a stream. It invokes the unchanged official
`Hc.initCookie` function to initialize normal browser metadata cookies
(`versionName`, `versionCode`, `vplatform`, `platformVersion`,
`deviceModel`, and `newLogin`) and clear legacy request-sequence cookies.
It does not initialize `pc_version`, change account identity or permissions,
or add an independently implemented cookie/signature algorithm.

The resolver accepts `{livepid, cnlid, stream, version, defn, adjust}`. Here
`livepid` is the channel PID and `cnlid` is its official stream ID. The normal
player passes `stream: "2"`, `version: "V1.0.0"`, and `adjust: 1` on its first
request. Use an explicit ordinary quality such as `defn: "hd"`; the server still
decides permissions. The Promise returns `currentUrl`, `definition`, `defn`,
`cdn`, `previewTime`, `previewAll`, `attemptNum`, `attemptAll`,
`playbacktime`, and `extraInfo`. Preserve preview and channel restrictions.

The official openapi and login SDKs provide the normal signature and account
state flow. `RJq7sO71JF.js` creates the global `Module` and fetches its sibling
`RJq7sO71JF.wasm`; this WASM produces the authorization ticket used by the
resolver. It is separate from media decoding. `cmg.worker.js`, despite its
filename, is loaded as a script to expose `CMGDecModule` and contains its CMG
decoder WASM in a data URI. Together with official `hls.cmg.js` / `Hls`, it is
used when attaching the returned stream to a video element. It does not replace
the ticket WASM. Exact dependency URLs are recorded in the vendor manifest.

The official SDK depends on the real browser origin and page context. This is
not a player that can be hosted at an arbitrary localhost or appassets origin:
those origins encountered CORS denial and upstream `ret=10012` in integration
checks. The intended integration obtains the real official main HTML normally,
retains its genuine context, blocks unnecessary site resources, and injects the
small application UI and resolver. It does not fabricate Origin or Referer,
rewrite the main-document response, change signatures, or bypass login, VIP,
region, copyright, platform, or DRM restrictions. Official service changes may
still invalidate this integration; playable channels must be verified at runtime.
