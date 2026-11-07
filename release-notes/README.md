# Release notes

One file per version, named after it: `0.1.0.json`, `0.1.0-beta.1.json`. A tag without it fails
before anything is built. The text is what the GitHub release page shows, German first, then English,
with the downloads under it (`node scripts/release-body.mjs <version>` prints the whole page).

```json
{
  "de": "- Kurze, verständliche Punkte\n- Was Leute merken, nicht wie es gebaut ist",
  "en": "- Short, plain points\n- What people notice, not how it's built"
}
```

## Releasing a version

1. Set the version in `Cargo.toml` (workspace), the `tauri.conf.json` of `apps/desktop` and
   `apps/setup`, and the `package.json` files.
2. Add `release-notes/<version>.json`.
3. Commit, tag `v<version>` and push both.

The tag starts `.github/workflows/installers.yml`. It checks that the tag, both `tauri.conf.json`
files and the notes agree, runs rustfmt, clippy and the tests on Windows, macOS and Linux, and builds
every download with `pnpm build:setup`: the Windows setups for x64 and ARM, the universal macOS disk
image, and for Linux x64 and arm64 the `.deb`, `.rpm` and portable `.tar.gz`. Each build is checked —
the setups unpack what they carry (`--check-payload`), the packages are installed and removed again,
the portable folder is unpacked and its libraries looked up. Then the release job collects all of
it, writes `SHA256SUMS.txt` and the Arch Linux `PKGBUILD` and creates a **draft** release with
these notes.

4. Look the draft over on GitHub (the files, the text) and publish it. A version with a suffix
   (`-beta.1`) is marked as a pre-release.

Publishing starts `.github/workflows/aur.yml`, which writes `uwusweep-bin`'s PKGBUILD and
`.SRCINFO` from the published `SHA256SUMS.txt` (`scripts/aur.mjs`) and pushes them to the AUR, once
the secret `AUR_SSH_PRIVATE_KEY` is set. Without it, the workflow says so and does nothing;
`node scripts/aur.mjs <version>` writes the same files to `target/aur/uwusweep-bin` for pushing by
hand.

To try the builds before tagging, push a branch named `ci/<anything>` or start **Installers** by
hand (Actions → Installers → Run workflow): everything is built and checked, the files are on the
run's page for 14 days, and no release is made.

## Signing and updates

Nothing is signed: there is no code-signing certificate for Windows (SmartScreen warns), no Apple
developer ID (macOS gets Apple's ad-hoc seal and asks once), and UwUSweep 0.1 has no updater, so
there is no update-signing key either. A new version is installed over the old one — setup or
package — and keeps the settings, the quarantine and the install traces.
