# Windows builds and Forgejo releases

## Local Windows build

The default `build-scripts/windows/build.ps1` creates the native executable,
portable ZIP and a per-user NSIS installer. Install Rust, MSVC/Windows SDK and
[NSIS 3](https://nsis.sourceforge.io/) first. NSIS must be on PATH or installed
at `%ProgramFiles(x86)%\NSIS`. No Python is required for the Windows build.

```powershell
.\build-scripts\windows\build.ps1 -Test -Check
.\build-scripts\windows\build.ps1 -NoInstaller
.\build-scripts\windows\build.ps1 -NoPortable
.\build-scripts\windows\build.ps1 -NoInstaller -NoPortable
```

Flags skip generating the selected output; they do not delete earlier output.
`-Debug` changes the application build profile. Local builds use Cargo.toml's
version. Local builds do not automatically sign or publish anything.

## Tagged CI/CD release

`.forgejo/workflows/release.yml` runs on pushes of `v*` and `V*` tags on the
`ubuntu-latest` runner. Valid tags use SemVer, e.g. `v0.1.0`, `V0.2.0` or
`v0.3.0-rc.1`. Invalid versions fail before compiling. Numeric version components
are limited to 65535 for Windows metadata. Prereleases are marked as such.

The job verifies that HEAD is the triggering tag/commit and derives the Cargo
manifest/lock versions in the job workspace. The GUI, executable VERSIONINFO,
manifest identity, installer and filenames then use that version. It does not
commit a version bump or change the source tag. The source archive includes the
derived manifest and lockfile so its version agrees with the binaries.

The runner needs outbound HTTPS, APT privileges (root or passwordless sudo),
Python 3.11+, git and sufficient space for LLVM and the Windows SDK. The job
installs the pinned Rust toolchain and cargo-xwin 0.23.1, LLVM, NSIS and GnuPG.
It builds **x86_64-pc-windows-msvc**, preserving the static CRT and native Win32 GUI.
There is no Linux application build. No Wine or Windows runner is required for
compilation; Windows runtime tests remain a separate acceptance requirement.

Both build paths reuse `build-scripts/windows/installer.nsi` and the same payload
layout. Ubuntu orchestration is `build-scripts/windows/release.py`; the existing
Windows build and clean entry points remain in their original directory.

`cargo-xwin` downloads Microsoft's CRT and Windows SDK and requires acceptance
of Microsoft's tool license. Review its [license notice](https://github.com/rust-cross/cargo-xwin#readme)
before enabling the workflow. These are build tools; no Qt runtime is introduced.

Configure these **Forgejo repository/organization secrets**:

| Secret | Purpose |
| --- | --- |
| `CI_KEY_B64` | Base64 of exactly one primary OpenPGP private key, including its signing subkey if used. |
| `CI_KEY_PASSPHRASE` | Passphrase for that key (empty is supported if the secret resolves empty). |
| `GH_KEY` | GitHub token with Contents read/write for `firebadnofire/NightLightAuto`, including tag push and release creation. |

Forgejo publication uses its short-lived automatic `${{ forgejo.token }}` for the
current repository. No additional Forgejo PAT is stored. Actions must be enabled
and its repository token must be allowed to create releases. Only trusted users
should be permitted to push release tags, since tag workflows can access secrets.

## Published artifacts and signatures

For each tag, CI produces a standalone application EXE, portable ZIP, setup EXE,
source ZIP, and `SHA256SUMS`. Each of those five files has an ASCII-armored detached
signature named `<filename>.asc`, including `SHA256SUMS.asc`. Signatures themselves
are not recursively signed. SHA256SUMS lists the four payloads; its signature
authenticates that list. All signatures are verified before uploading.

Keys are imported into a temporary GnuPG home, passphrases go through stdin,
and signing secrets are scoped to the signing step. No key is written to the
repository or release. Publish your public key through your trusted distribution
channel so users can authenticate the signing fingerprint. These signatures are
**OpenPGP signatures, not Authenticode**; they do not establish a Windows code-signing
certificate or suppress SmartScreen.

The GitHub tag is pushed without force, preserving the original tag/commit. A
conflicting existing tag fails. CI stages both releases as drafts and uploads the
same signed files, then publishes them. A failure does not silently skip the mirror.
Two services cannot be committed atomically; a final publication failure can leave
one published and the other draft. Inspect both release pages before retrying.
Draft uploads can be replaced on retry; public assets are never overwritten.
Existing GitHub assets with matching SHA-256 API digests can be skipped; otherwise
the job fails safely rather than altering an already published artifact. Keep the
original signed files to recover a partially published release; a fresh signing run
can produce different signature bytes. Use a new version tag for changed binaries.

The installer uses HKCU and the current user's Programs/Start Menu. Uninstall
preserves user configuration, recovery records and logs, and deletes only known
installed files. Upgrade/uninstall request graceful Exit rather than killing the
process. Validate actual install/upgrade/uninstall on Windows before release
certification, including running-app and startup-registration cases.

## Verification

```sh
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum --check SHA256SUMS
gpg --verify FullscreenNightLight-0.1.0-windows-x64-setup.exe.asc FullscreenNightLight-0.1.0-windows-x64-setup.exe
```

Release tooling tests: `python3 -m unittest discover -s build-scripts/windows -p test_release.py -v`.
Do not use production secrets in local tests or create a test release on the public
repositories. Read `docs/VALIDATION.md` for the checks actually performed.
