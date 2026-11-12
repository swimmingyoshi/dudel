# Releasing Dudel

A release is a portable Windows ZIP and a matching source ZIP, each with a
SHA-256 file, published as a GitHub release. Linux tarballs are experimental and
built separately.

GitHub Actions minutes are limited: verify locally, push code in batches and run
hosted CI once on the final candidate.

## Steps

1. **Version and notes.** Set the version in `Cargo.toml` (and run a build so
   `Cargo.lock` follows). Move the *Unreleased* entries in
   [RELEASE_NOTES.md](../../RELEASE_NOTES.md) under the new version and date;
   update the [quick start](../QUICK_START.md) and
   [format support](../FORMAT_SUPPORT.md) where behavior changed.
2. **Verify.** Run `.\scripts\Verify.ps1 -Mode Full` on the frozen source.
   See [testing](TESTING.md).
3. **Candidate package.** Run `.\scripts\Package.ps1`. It builds both binaries
   with path remapping, the source ZIP and the third-party notices, and refuses
   personal paths.
4. **Accept the binaries.** Record the executable and DLL SHA-256 values in
   [RELEASE_ACCEPTANCE.json](../release/RELEASE_ACCEPTANCE.json), and note the
   evidence and limits in the [release decision](../release/RELEASE_DECISION.md).
5. **Release package.** Run `.\scripts\Package.ps1 -Release`. It refuses
   binaries that differ from the accepted hashes. Check the package with
   `scripts/check_source_archive.py` and `scripts/check_release_notices.py`.
6. **Push and CI.** Push `master`, wait for the Windows workflow to pass, then
   create an annotated tag `vX.Y.Z` on that commit and push it.
7. **Publish.** Create a draft GitHub release from the tag, upload both ZIPs
   and their `.sha256` files, download them again to compare hashes, and
   publish the draft. Refer to releases by id in scripts: tags can move.
   Keep the asset names exactly as `Package.ps1` wrote them: **Help > Check
   for updates** in earlier versions finds the package by its name (see
   [DISTRIBUTION.md](../DISTRIBUTION.md#installing-and-updating)).
8. **Linux (optional).** On a machine with `cargo-zigbuild`, run
   `python scripts/package_linux.py` and upload the tarball to the same release.
