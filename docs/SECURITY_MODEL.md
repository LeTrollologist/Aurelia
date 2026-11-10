# Security model

## Trust boundaries

| Layer | Guarantee |
| :--- | :--- |
| `aurelia-core`, `aurelia-render`, `aurelia-io`, `aurelia-ui` | safe Rust |
| `aurelia-app` Win32/GDI/COM calls | `unsafe` FFI reviewed as a boundary |
| MSVC C runtime | platform code; outside Rust safety guarantees |
| Third-party decoders (`image`, `rfd`, `egui`) | upstream crates; versioned and audited where possible |
| `tools/release` | developer tooling, not shipped |

“Memory-safe Rust” means the application logic is written in safe Rust. It does **not** mean
every instruction executing in the process is memory-safe; the Win32/CRT/FFI layers are not.

## Network policy

- The application makes no outbound network connections, has no telemetry, and does not check
  for updates.
- Discord Rich Presence is **off by default**. When enabled it talks only to the local Discord
  client over `\\.\pipe\discord-ipc-*`; it uses no token, bot, webhook or remote endpoint.
- Presence diagnostics do not record project names or paths. File logging is **off by default**;
  setting `AURELIA_PRESENCE_TRACE=1` opts into a local `presence.log` containing only generic
  state (`enabled`, `show_project_name`, `painting`, connection status). A disabled presence writes
  no update line at all, and startup removes a legacy `presence.log` left by older builds when
  tracing is off.
- The release tooling contacts GitHub and (optionally) VirusTotal; it is not part of the app.

## Input handling

- AUR parsing validates the magic, dimensions, compressed and decompressed lengths, format tags
  and masks. Payloads are bounded to the expected size; malformed files return an error.
- Decoding is also bounded across the whole document: at most 4096 layers, 512 Mi pixels, and
  2 GiB of decoded layer storage (pixels, masks, text and metadata). The budget is reserved before
  each buffer is allocated, so a small file cannot request an unbounded total by multiplying many
  layers.
- Parent ids and layer names are validated as part of loading/saving. Duplicate ids, missing or
  non-group parents, self-parents, cycles and chains deeper than 1024 parent links are rejected
  before a `Document` is returned, and names longer than 65,535 bytes are rejected on save rather
  than truncated.
- Win32 clipboard text reads are bounded by `GlobalSize`; a block without a NUL terminator inside
  its allocation is rejected. Clipboard allocations that fail or whose ownership is not
  transferred are released.
- Discord IPC frames are capped at 1 MB in both polling and the READY handshake. READY must be a
  FRAME (opcode 1) and its body must arrive before the handshake deadline.
- Image imports use the `image` crate decoders.
- There is no plugin system and no executed project content.

## Release integrity

- Artifacts and `SHA256SUMS.txt` are Ed25519-signed; the public key is published in the repo and
  with the release.
- The installer is **not Authenticode-signed**. This is a permanent distribution constraint, not
  a pending release task: no public certificate path is available, and a self-signed certificate
  would not establish SmartScreen trust. SmartScreen reputation is therefore separate from the
  Ed25519 artifact signature.
- VirusTotal is advisory. Unsigned binaries frequently trigger heuristic/ML detections; a clean
  scan is not proof of safety.
- The private signing key is local and gitignored; see
  [RELEASE_PROCESS.md](../RELEASE_PROCESS.md) for rotation and loss procedures.

## Reporting

Follow [SECURITY.md](../SECURITY.md): report privately to the maintainer rather than opening a
public issue.
