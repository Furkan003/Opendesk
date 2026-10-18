# Opendesk transport version 2

The connection uses TCP and Noise
`NNpsk0_25519_ChaChaPoly_BLAKE2s`. The rendezvous relay forwards encrypted
records and receives only a SHA-256 routing identifier, never the pairing
secret. Both peers exchange an encrypted protocol version and capability mask
before the host receives an approval request. Version 1 applications cannot
connect to version 2 applications.

Generated pairing codes contain 20 independently random base-32 symbols: 100
bits of entropy, displayed in four groups of five. Matching is case insensitive
and ignores hyphens. These are not six-digit PINs. Legacy 64-character
hexadecimal pairing secrets are also accepted. An invite is private connection
information; possession of its pairing secret permits requesting a session.

## Host permission boundary

The host explicitly grants screen, input, clipboard, files, and audio
permissions. The transport enforces them on outgoing and incoming messages.
The host can change them while connected; revoking input releases held input,
and revoking screen/files cancels the corresponding active network buffers.
Already transmitted packets for a revoked capability are discarded. Screen
permission also controls remote monitor selection and video feedback.

Unattended password/TOTP requests travel inside the encrypted connection,
before approval. At most three requests are accepted per session. The network
does not verify or automatically approve them: the host's local unattended
policy makes that decision. Authentication requests redact their Debug output.

## Files and folders

The streaming transfer protocol sends a manifest with relative paths, file
sizes, and SHA-256 checksums. The receiver must approve a local destination
before file content is requested or written. File contents are streamed in
12 KiB chunks with bounded network queues, interleaved with desktop frames.

Incomplete content remains in the selected destination under
`.opendesk-partials/<manifest-id>/`. To resume, resend the same unchanged source
and accept the same destination. The receiver requests offsets and hashes of
existing prefixes; the sender verifies those hashes before seeking. The
receiver verifies full checksums before creating final files without
overwriting existing destinations. An existing final file is skipped only if
its size and checksum match the manifest.

Limits: 128 regular files per transfer, 240 UTF-8 bytes per relative path, and
1 TiB total metadata size limit for file contents. Empty files are supported;
empty directories, symlinks, Windows reparse points, and special files are not
transferred. Reserved Windows names and path traversal are rejected. The
older in-memory file API remains limited to 16 MiB.

## Media and session health

The transport carries JPEG frames, monitor descriptions and selection, and
viewer feedback for frame rate and JPEG quality. The desktop chooses the
capture/encoding settings. This is not a hardware video codec implementation.
PCM audio packets support one or two channels at 8–48 kHz, up to 4096 signed
16-bit samples per packet. Audio capture/playback availability depends on the
platform backend and granted permissions.

Connections send an encrypted keepalive every 20 seconds and close after 90
seconds without incoming data. Reconnection creates a new encrypted session;
host approval or the configured unattended policy is required again. Transfer
resumption uses the explicit procedure above.

Loopback tests cover authentication failures, version rejection, consent,
live permission revocation, audio and monitor messages, simultaneous large
file/frame traffic, and streaming folder transfers resuming a verified partial
file. These do not replace acceptance tests on two physical computers.
