# Windows / Linux protocol validation

Tested 2026-10-18 with Windows x86_64 and Ubuntu 26.04 x86_64 under WSL2,
using separately compiled Opendesk 0.2.0 diagnostic executables. This exercises
two operating-system kernels on the same computer. It is not a test of two
physical desktops, real screen capture, remote input, or Internet deployment.

The explicit `opendesk-probe` diagnostic auto-approves view-only access to
synthetic test data. It has a 15-second deadline. It sends and decodes a known
2×2 JPEG, exchanges known chat and file payloads in both directions, checks
their contents, and requires a final acknowledgment before closing. It never
captures the desktop or injects input. Tests used a temporary generated
100-bit pairing code and the encrypted version-2 protocol.

| Connection | Result |
|---|---|
| Windows viewer → Linux host, direct WSL network address | Passed; both processes exited 0 |
| Linux viewer → Windows host, direct Windows bridge address | TCP connection timed out; no session established |
| Linux viewer → Windows host, through a temporary Linux relay | Passed; both desktop-role processes exited 0 |

The direct Windows inbound attempt listened on the Windows/WSL bridge
interface at `172.31.128.1:45688`; the Linux TCP connect timed out. No firewall
or network policy was changed, so the exact blocking policy was not diagnosed.
The Windows diagnostic listener terminated at its deadline.

The relay test used only the WSL network interface, with a 25-second process
timeout. Windows initiated its connection to that local relay, allowing the
Windows host / Linux viewer roles to be tested without changing inbound
Windows access. The relay forwarded ciphertext; the JPEG decoding, chat and
file checks happened in the Windows and Linux desktop-role processes.

Representative successful outputs:

```text
PASS os=windows role=viewer jpeg_decoded=true chat=true file=true input=false
PASS os=linux role=host jpeg_decoded=false chat=true file=true input=false
PASS os=linux role=viewer jpeg_decoded=true chat=true file=true input=false
PASS os=windows role=host jpeg_decoded=false chat=true file=true input=false
```

Build the diagnostic with `cargo build --bin opendesk-probe` using desktop
features. Its modes are `--host`, `--connect`, `--host-relay`, and
`--connect-relay`, followed by an address and an ephemeral pairing code.
It is a diagnostic utility, not a replacement for the application's normal
host approval workflow. Acceptance tests on real Windows and Linux desktop
endpoints remain necessary.
