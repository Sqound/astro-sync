# Security policy

Please do not publish an exploitable vulnerability in a public issue. Use the
repository's private **Report a vulnerability** feature, or write to
[dev@sqound.jp](mailto:dev@sqound.jp), with the affected version, the
conditions of exploitation and a minimal reproduction without personal data.

## Security model

- Astro Sync only reads the folders you associate, when you start an import;
  it never writes, moves or deletes a file in them.
- The desktop application keeps the credential of a portal (a device
  token, which the portal's owner can detach at any time) in the operating
  system's credential store (the macOS Keychain, the Windows Credential
  Manager), and in
  a file readable by you alone only when no keychain is available.
- Uploads go only to the portal you configured. Prefer its https address;
  use plain http only for a portal of your own local network.
