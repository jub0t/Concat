# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io/),
certificate by [SignPath Foundation](https://signpath.org/).

Concat has applied to SignPath Foundation. Until the application is accepted,
Windows releases are not signed; once it is, every Windows release is signed
under this policy.

## What is signed

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
merged. Every member uses multi-factor authentication on GitHub and on
SignPath.

## Privacy policy

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or operating
it. Concat reaches the network only when you ask it to:

- **Check for updates** in Settings asks GitHub for the list of releases, and
  installing one downloads it from GitHub.
- **Downloading a model** (transcription, speech, cutout) fetches it from the
  address shown in Settings.
- **The Remote API** is off by default. Turned on, it listens on this computer
  (`127.0.0.1`) unless you give it another address, and answers only callers
  that hold its token.

Nothing is sent about you, your projects or your media, and there is no
telemetry.
