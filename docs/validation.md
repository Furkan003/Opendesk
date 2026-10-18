# Validation status

## Verified locally

- Rust Windows compilation, including the native eframe application.
- Windows real display capture: 1920x1080, JPEG encode and decode successful.
- Loopback encrypted transport: valid key, invalid key rejection, explicit
  consent, blocked pre-consent payloads, view-only input restriction,
  input messages, multi-record frame/file transfers, chat and clipboard.
- Malicious authenticated pre-consent input is rejected.
- Relay raw byte forwarding and complete encrypted approved relay session.
- File path validation and refusal to overwrite existing files.
- JPEG malformed input and image dimension limits.
- Linux Ubuntu 26.04 cargo check for all targets passed using a private sysroot.

The final delivery message records the final test count and build results.

## Required before claiming production or full platform parity

- Windows host -> Linux X11 viewer and reverse direction on distinct machines.
- Real remote pointer, drag, keyboard layout, modifier and focus-loss tests.
- Session stop during slow network/file transmission; power loss/reconnect.
- Internet relay across NATs, firewall deployment, abuse/load testing.
- Multiple-monitor DPI, screen lock, UAC, protected desktops and permissions.
- Linux Wayland portal sessions, system audio and Android physical-device sessions (implemented in 0.2, still require hardware validation).

Unit and loopback tests do not establish a real two-computer GUI session or
all AnyDesk feature parity. No unattended service is installed automatically.

## Final checks for 0.1.0

- Windows test suite: 18 passed (15 library, 3 image-decoding tests).
- Concurrent 2 MiB frame plus two 2 MiB file transfers complete without deadlock.
- Windows rustfmt check and Clippy with -D warnings passed.
- Windows release executable: version, 1920x1080 capture/JPEG round trip,
  and hidden native GUI initialization passed before final packaging.
- Linux Ubuntu/WSLg: source and native binaries build; version and hidden GUI
  initialization pass. WSLg host capture failed, now reported as unsupported
  Wayland hosting. Physical Linux X11 capture/input is still unverified.

## 0.2 Android design validation

- Flutter analyze passed with no issues.
- Both production widget tests passed, including TR/EN and dark theme controls.
- Phone-width light/dark widget renders were inspected; bundled fonts and icons
  render correctly. These are renderer previews, not physical-device evidence.
- ARM64 release-preview APK built and APK v2 signature verification passed.
- APK's Rust core matches the stripped native build artifact; merged native
  library matches the current Rust ARM64 release output.
- See README for Android host feature limits and cross-os-validation.md for
  the exact Windows/Linux transport checks.

## 0.2 desktop design validation

- 36 Rust tests passed (28 library and 8 application tests).
- All-target strict Clippy and rustfmt checks passed.
- Native 1080x760 renderer previews for home/devices/settings and light/dark
  themes were inspected. Slider contrast, navigation and invite fields checked.
- UI capture diagnostics run hidden and close automatically, without starting
  a remote session. Preview images are under docs/previews/.

## 0.2 Windows release checks

- Release executable reports Opendesk 0.2.0 and embeds the Opendesk icon/name.
- Real local display capture and JPEG round trip passed at 1920x1080.
- Hidden native GUI initialization passed.
- System audio capture and playback initialization passed. This checks device
  initialization, not an audible end-to-end remote sound session.

## Ubuntu 24.04 physical computer, 2026-10-18

A separate x86_64 Ubuntu 24.04.4 LTS computer was accessed with the owner's
permission over the local network. Its XFCE/X11 desktop was opened through
RDP (`DISPLAY=:10`). The compatibility build's required GLIBC versions are
at most 2.39; the build uses private Ubuntu 24 libm/ALSA link inputs rather
than modifying the target's system libraries.

- Opendesk 0.2.0 version command passed.
- Real X11 capture/JPEG round trip passed at 1920x1080 (118618 JPEG bytes).
- Native GUI initialization passed, exit 0.
- The redesigned application opened in the real XFCE session.

This establishes X11 capture and GUI startup on a separate physical machine;
it does not establish a physical Wayland session or an audible remote audio
session. Interactive connection results are recorded separately below.

## Code-only Internet sessions (iroh), 2026-10-18

- `cargo test --lib p2p -- --ignored` passed on Windows: host and viewer found each
  other by pairing code only through n0 address lookup and relays.
- Cross-machine probe: the Ubuntu 24.04.4 computer hosted with
  `opendesk-probe --host-p2p - CODE`; this Windows computer connected with
  `--connect-p2p - CODE`. Both reported PASS (Noise session, JPEG frame, chat,
  file); no IP address was entered. The path started on a public relay and an
  earlier run upgraded to a direct LAN path.
- Not yet verified: Android phone sessions, a remote network (e.g. USA) and
  CGNAT/mobile-carrier networks, and long-session throughput.
- Linux release binaries were built natively on that Ubuntu 24.04.4 machine
  with `CARGO_PROFILE_RELEASE_LTO=off`; thin LTO hit an lld
  "undefined hidden symbol" error in egui there. Windows keeps thin LTO.
- Android emulator (API 36.1 x86_64 image running the ARM64 APK through
  ARM translation): the app connected to a Windows `--host-p2p` probe by code
  only, reached the session screen and rendered the probe's frame. The path
  started on a public relay (~1 s) and switched to direct (14 ms). The probe
  itself reports FAILED because it expects a probe peer's chat/file replies;
  the app does not send those. iroh warns that Android DNS settings need an
  NDK JNI context and falls back to its default resolvers. Physical phones and
  mobile-carrier networks remain untested.
