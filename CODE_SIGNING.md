# Code signing policy

**Windows releases are not code-signed yet.** Windows shows a SmartScreen
warning before an unsigned program runs (**More info** › **Run anyway**), and
where Smart App Control is on, it refuses to run it at all. Once Concat has a
signing certificate, every Windows release is signed under this policy.

## What will be signed

The Windows program (`concat.exe`) and its installers (the setup `.exe` and the
`.msi`), built from this repository. Libraries Concat ships but does not build
from its own source, such as FFmpeg's and sherpa-onnx's DLLs and Microsoft's
ONNX Runtime and Visual C++ runtime, are not signed with this certificate; they
carry their own maintainers' signatures where they have them.

## How a signed release is made

- Every signed file is built by GitHub Actions from a tagged commit of
  <https://github.com/jub0t/concat>, by the workflows in
  [`.github/workflows`](.github/workflows) (`release.yml`, which calls
  `build-app.yml`). Nothing built on a person's machine is signed.
- A release is built only after the tests pass on the tagged commit, and each
  Windows bundle is launched before it is uploaded (see
  [`scripts/ci`](scripts/ci)).
- Every signing request is approved by hand by an approver, below.
- The workflows and build scripts are part of the source and are reviewed like
  any other change.

## Team and roles

| Role | Members |
|---|---|
| Committers and reviewers | [@jub0t](https://github.com/jub0t) |
| Approvers | [@jub0t](https://github.com/jub0t) |

Committers may change the source without further review. Every change from
anyone else, such as a pull request, is reviewed by a reviewer before it is
merged. Every member uses multi-factor authentication on GitHub.

## Privacy policy

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or operating
it. Concat reaches the network only when you ask it to:

- **Settings › Version** asks GitHub for the list of releases when you open
  it or press **Check for updates**, and installing one downloads it from
  GitHub.
- **Downloading a model** (transcription, speech, cutout) fetches it from the
  source picked under **Downloads** in Settings › General, and tries the other
  sources listed there if that one fails.
- **The Remote API** is off by default. Turned on, it listens on this computer
  (`127.0.0.1`) unless you give it another address, and answers only callers
  that hold its token.

Nothing is sent about you, your projects or your media, and there is no
telemetry.
