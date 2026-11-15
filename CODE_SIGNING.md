# Code signing policy

**Windows releases are not code-signed yet.** Windows shows a SmartScreen
warning before an unsigned program runs (**More info** › **Run anyway**), and
where Smart App Control is on, it refuses to run it at all. Once Concat has a
signing certificate, every Windows release is signed under this policy.

## What will be signed

Everything in the Windows download: the program (`concat.exe`), its installers
(the setup `.exe` and the `.msi`), and the libraries shipped beside it. Smart
App Control checks every library a program loads, so a signed program with
unsigned libraries is still refused.

## How a release is made

- Every released file is built by GitHub Actions from a tagged commit of
  <https://github.com/jub0t/concat>, by the workflows in
  [`.github/workflows`](.github/workflows) (`release.yml`, which calls
  `build-app.yml`). Nothing built on a person's machine is released.
- A release is built only after the tests pass on the tagged commit, and each
  bundle is launched before it is uploaded (see [`scripts/ci`](scripts/ci)).
- The workflows and build scripts are part of the source and are reviewed like
  any other change.

## What Concat sends over the network

Concat reaches the network only when you ask it to:

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
