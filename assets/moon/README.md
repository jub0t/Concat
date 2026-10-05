# `.moon` bundles https://moonpkg.github.io/cli / https://github.com/moonpkg/cli

A `.moon` is one file that carries a whole app: the program, its manifest, its
menu entry and its icon. A person installs Concat with

```
moon install Concat-0.2.6-linux-x86_64.moon
```

and gets the editor on a machine through moon tool.

## Where it comes from

The bundle is built from the same staged folder as the `.deb`, the `.rpm` and the AppImage - the one `build-app.yml` makes in its "Stage (Linux)" step - so all four ship the same binary and the same libraries.
Nothing is built twice and nothing can drift:

```
build-app.yml  "Stage (Linux)"      stage/Concat-<v>-linux-<arch>/
                "Package (Linux)"   .deb, .rpm, .AppImage from that folder
                                     .moon from that folder
```

The step that packs it is `assets/moon/build-moon-bundle.sh`.

## What is inside

```
Concat-0.2.6-linux-x86_64.moon  # (tar.gz)
├── concat.manifest             # moon's key=value manifest, paths relative
├── app/
│   ├── concat                  # the program
│   ├── lib/*.so*               # FFmpeg and ONNX Runtime, beside it
│   ├── LICENSE                 # AGPL, and the notices for the libraries
│   └── THIRD_PARTY_NOTICES.md
├── desktop/concat.desktop      # the menu entry
└── icon/concat.png             # the icon
```

`lib/` stays beside the binary *inside* `app/` because that is where the
program looks for it.

The manifest is the same format moon writes for every install, carrying the
paths a bundle can:

```
dir=app
main=app/concat
version=0.2.6
desktop=desktop/concat.desktop
link=concat
to=app/concat
cmd=concat
```

`moon install` copies `app/` to `~/.local/share/moon/apps/concat/`, links
`concat` onto the path, and rewrites the bundled menu entry to this machine's
paths.

## Building one

The release does it on every tag, once per Linux architecture. By hand, with a
staged folder of your own:

```
assets/moon/build-moon-bundle.sh --stage stage/Concat-0.2.6-linux-x86_64
```