# Contributing to Astro Sync

Thank you for helping. Please keep changes focused and the checks green:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
cd app && npm ci && npm run build && npm test
```

- **Source files are never touched.** Astro Sync reads the folders you
  associate and never writes, moves or deletes a file in them; a change that
  needs to do so is out of scope.
- **Data**: never commit personal data: your images, your logs, the address of
  your portal, coordinates of a real site, credentials. Fixtures are synthetic.
- **The portal's contracts** (`contracts/`) are copies of the API contracts of
  the Astro Portal repository (`packages/astro-contracts`): to change a
  contract, propose the change there, not here.
- **Interface text** exists in the ten languages of the application
  (`app/src/i18n/`); a new string is added to all of them. Texts address the
  person politely (_vous_ in French).
- Describe the problem the change solves in the pull request, and add or adjust
  a test.

By contributing you agree that your contribution is licensed under the
project's license, MIT.
