# Linux build verification

Verified on 2026-10-17 using Ubuntu 26.04 LTS under WSL2, x86_64,
Rust 1.98.1, and the committed Cargo.lock.

`cargo check --all-targets` passed for the application, relay binary, and tests.
This verifies compilation; a Windows-to-Linux graphical session has not yet been
tested. Linux screen capture and input should initially be tested in an X11
desktop session. Wayland compositor permissions and capture APIs differ, so a
successful build does not establish Wayland remote control support.

## Normal Linux development machine

Install Rust using the normal Rust toolchain for your machine. On Debian/Ubuntu,
the native development packages include:

```sh
sudo apt-get install build-essential pkg-config libdbus-1-dev libxdo-dev \
  libx11-dev libxrandr-dev libxcb-randr0-dev libxkbcommon-dev
cargo check --all-targets
cargo build --release --bins
```

Run the desktop application from `target/release/opendesk`; run the optional relay
from `target/release/opendesk-relay`. Runtime shared-library versions must match
the distribution used for the build. This build is not a portable Linux bundle.

## Verification performed on this computer

No system packages were installed or replaced. Existing X11/XCB/XRandR/XKB
development libraries were used. Missing Ubuntu packages `libdbus-1-dev`,
`libxdo-dev`, and `libxdo3` were downloaded with `apt-get download` and extracted
with `dpkg-deb -x` into the workspace's `work/linux-sysroot` directory. Package
metadata was adjusted to point at this private directory. `PKG_CONFIG_PATH`,
`LIBRARY_PATH`, and `LD_LIBRARY_PATH` were set only in the build process.

Build products remain in `work/opendesk-linux-target`; the helper script is
`work/build-linux.sh` in the enclosing workspace. The existing desktop and
system settings were not modified.

The dependency `screenshots 0.8.10` produces a Rust future-incompatibility warning;
it does not prevent the current toolchain from checking this project.

## Runtime checks in WSLg

The Linux binary reports Opendesk 0.1.0 and its hidden GUI initialization test
passed. A screen-capture attempt in WSLg Wayland failed in the screenshots
provider; forcing X11 under WSLg returned an XCB protocol error. Opendesk now
rejects Wayland hosting with a clear error and contains capture-provider panics.
A physical Linux X11 host capture and remote input session remain unverified.
Wayland viewing does not need local screen capture, but full Windows/Linux
sessions still require real endpoint acceptance tests.

## Version 0.2.0 package, 2026-10-18

The final 0.2.0 source passed `cargo check --all-targets` and
`cargo build --bins` on Ubuntu 26.04. Stripped development binaries are in
`dist/linux-x86_64`, also available as `Opendesk-linux-x86_64-0.2.0.tar.gz`.
The package contains the desktop, relay, setup, updater, publisher signing
utility and synthetic diagnostic probe. It contains no private signing keys.

`ldd` found no missing libraries when using the bundled `libxdo.so.3`; ALSA
resolved to the existing system `libasound.so.2`. The libxdo copyright notice
is included. The launchers set a process-local library search path.

The packaged `--version` returned `Opendesk 0.2.0` and the packaged hidden
`--smoke-test` initialized the WSLg GUI and exited 0. Mesa/EGL driver warnings
appeared before the successful fallback. No portal screen-capture or remote
input test was performed during packaging. Wayland capture additionally needs
the desktop portals, PipeWire and GStreamer PipeWire/JPEG plugins; those system
services are not bundled. See `cross-os-validation.md` for the separately
verified Windows/Linux synthetic protocol exchange.

## Native interface refresh and Ubuntu 24.04 compatibility

The refreshed native interface embeds IBM Plex fonts and application icons.
The font license is included as `licenses/IBMPlex-OFL.txt` in the Linux bundle.
The refreshed source again passed Linux checking for all targets.

Linking directly on Ubuntu 26.04 selected `atan2f` and `acosf` at GLIBC 2.43,
which cannot run on Ubuntu 24.04. The final executables were relinked using a
private copy of Ubuntu 24.04's `libm` and ALSA runtime libraries. No system
libraries were replaced, and those compatibility libraries are not bundled.
The resulting main executable requires GLIBC versions up to 2.39.

On the user's separate Ubuntu 24.04.4 X11 desktop, the compatible executable
reported Opendesk 0.2.0, captured a 1920×1080 JPEG (118618 bytes), and passed
the hidden GUI initialization test. All three commands exited 0. This verifies
the actual machine's loader, capture backend and GUI startup; full session,
input, audio and Wayland flows need their own acceptance results.

All six refreshed package executables were checked for required GLIBC symbol
versions; each requires 2.39 or earlier. Package dependency inspection found no
missing shared libraries on the build machine. The final packaged native GUI
with the embedded fonts/icons again passed the isolated WSLg startup smoke
test. Only libxdo is bundled; target systems provide their own glibc, libm,
ALSA and desktop services.
