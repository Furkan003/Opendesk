# Signed releases and installation

The full Windows bundle contains the app, relay, updater and setup. Extract the
whole ZIP before starting setup. Setup copies to the selected user directory;
shortcut and visible autostart are opt-in. It does not install a hidden service,
open firewall ports or start listening. Existing files are not overwritten.

Updates currently replace the application executable only. Changes to updater,
relay, shared libraries or platform integrations require a new full bundle.
There is no default hosted update feed; an operator provides its HTTPS address.
Installing and applying updates were not performed on the development machine.

## Publisher signing

The application pins the Ed25519 public key in `assets/update-public-key.txt`.
The private key is intentionally outside the deliverable tree. It is required
to publish a future update that existing clients will trust. Replacing the pin
requires rebuilding and distributing the application through a trusted channel.

From a publisher workspace containing a private `work/` directory:

```sh
opendesk-sign-release --keygen work/private-key.txt PUBLIC-KEY.txt
opendesk-sign-release --sign work/private-key.txt manifest.json signed-manifest.json
```

Do not overwrite the existing pin when publishing updates for this build. Keep
private keys outside source control and never place them on the public server.
The supplied signing tool creates new keys without overwriting an existing key.

The unsigned manifest fields are:

```json
{
  "version": "0.3.0",
  "platform": "windows-x86_64",
  "asset_url": "https://YOUR-SERVER/opendesk.exe",
  "sha256": "REPLACE_WITH_64_HEX_DIGITS",
  "size": 1234567
}
```

The version, hash and size must describe the actual new executable. The example
is a format illustration, not an existing release. Linux platform is
`linux-x86_64`. The signed envelope contains base64 payload bytes and signature;
its exact payload bytes are verified before parsing. Unknown manifest fields,
wrong signatures/platforms, older versions, invalid sizes and hash mismatches
are rejected. Redirects are disabled. HTTP is allowed only on loopback for tests.

Check and download run in background threads. Applying is an explicit UI action;
the helper verifies the staged update again, waits for the running app to close,
keeps a rollback copy during replacement and restarts the application. A new
signed manifest alone does not deploy any external infrastructure.
