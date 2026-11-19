# Third-party sources

Compiled into `concat-stretch` by `build.rs`. Nothing here is changed from
upstream; each folder keeps its licence file.

| Folder | Upstream | Commit | Licence |
|---|---|---|---|
| `bungee/` | https://github.com/bungee-audio-stretch/bungee | 61f52bf34be577d189d388772ddfa3696dee77d8 (2026-10-18) | MPL-2.0 (`bungee/LICENSE`) |
| `pffft/` | https://bitbucket.org/jpommier/pffft | 02fe7715a5bf8bfd914681c53429600f94e0f536 | FFTPACK (BSD-like), in the head of `pffft.c` |
| `eigen/` | https://github.com/eigenteam/eigen-git-mirror | c29c800126982c561e8d0b9255dc65474cd98de3 (3.4.90) | MPL-2.0 (`eigen/COPYING.MPL2`) |

`bungee/` holds the library only: `bungee/Bungee.h`, `bungee/Modes.h` and
`src/`, without the command-line tool or the streaming helpers Concat does
not use. `eigen/` holds only the 142 headers that Bungee's sources include
on x86_64 (SSE, AVX) and arm64 (NEON), found with `clang++ -MM` on each
source for each target.

To update: clone Bungee with its submodules at the new commit, copy the
same files over, list Eigen's headers again with `-MM`, and change the
table above.
