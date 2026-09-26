{
  # Concat on Nix, Linux only: the people asking for a flake run Linux, and
  # the macOS and Windows builds ship from release.yml.
  #
  #   nix build            the editor window, ./result/bin/concat
  #   nix run              build and launch it
  #   nix develop          a shell with everything `cargo run -p concat` needs
  #
  # The build is one Cargo workspace under src/, so cargo dependencies
  # come straight from src/Cargo.lock and there is no vendor hash to keep
  # in sync. Three native pieces need care inside the sandbox, which has no
  # network:
  #
  # - FFmpeg is linked, not spawned. nixpkgs' ffmpeg_8 provides the headers
  #   and libraries; bindgen finds them through pkg-config and bindgenHook's
  #   libclang.
  # - whisper.cpp is compiled in by cmake from the source vendored inside
  #   whisper-rs-sys, so it needs cmake and a C++ toolchain and nothing else.
  # - sherpa-onnx (text to speech) links prebuilt static libraries that its
  #   sys crate would download at build time. They are fetched here as fixed-
  #   output derivations instead and handed over with SHERPA_ONNX_ARCHIVE_DIR.
  # - ONNX Runtime (the cutout models) is likewise downloaded by its sys
  #   crate. nixpkgs' onnxruntime is linked instead, through ORT_LIB_LOCATION,
  #   dynamically, so the wrapper's rpath finds it.
  # - Skia, the window's renderer as on every other platform, links prebuilt
  #   binaries that skia-bindings would download at build time. They are
  #   fetched here too and handed over as a file:// SKIA_BINARIES_URL; with
  #   a stale key or version the build falls back to compiling Skia from
  #   source, which the sandbox cannot do, so a mismatch fails loudly.
  #
  # Vulkan is loaded at run time, hence the LD_LIBRARY_PATH on the wrapper.
  description = "Concat - free and open source video editor";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      eachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Must match the sherpa-onnx-sys version in src/Cargo.lock: the sys
      # crate names the archive after its own version and refuses any other.
      sherpaVersion = "1.13.7";
      sherpaArchives = {
        x86_64-linux = {
          name = "sherpa-onnx-v${sherpaVersion}-linux-x64-static-lib.tar.bz2";
          hash = "sha256-0b56aawrMBIAWNgwLmJCOaMGQIU4PPpHmUoU/cRMMtY=";
        };
        aarch64-linux = {
          name = "sherpa-onnx-v${sherpaVersion}-linux-aarch64-static-lib.tar.bz2";
          hash = "sha256-dDtN66urLp9lTjk9pyY72Xlaq+1groDIMCMFQINBXRQ=";
        };
      };
      sherpaArchiveDir =
        pkgs:
        let
          archive = sherpaArchives.${pkgs.stdenv.hostPlatform.system};
        in
        pkgs.linkFarm "sherpa-onnx-archives" [
          {
            name = archive.name;
            path = pkgs.fetchurl {
              url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v${sherpaVersion}/${archive.name}";
              inherit (archive) hash;
            };
          }
        ];

      # Must match skia-bindings in src/Cargo.lock: the archive is named
      # after the crate version (the tag) and a key of rust-skia's commit,
      # the target and the Skia features i-slint-renderer-skia turns on for
      # Linux (gl, vulkan, plus skia-safe's defaults). A Slint or skia-safe
      # bump changes these; the new name is printed by skia-bindings' build
      # script as "TRYING TO DOWNLOAD AND INSTALL SKIA BINARIES: tag/key".
      skiaVersion = "0.153.3";
      skiaKeyFeatures = "ganesh-gl-jpegd-jpege-pdf-vulkan";
      skiaArchives = {
        x86_64-linux = {
          key = "b7f043e0b1e2a850e702-x86_64-unknown-linux-gnu-${skiaKeyFeatures}";
          hash = "sha256-nr5MRIzJ94m60kHL0Rc5zfdZ4n21cZwjp2NeCW1YB7M=";
        };
        aarch64-linux = {
          key = "b7f043e0b1e2a850e702-aarch64-unknown-linux-gnu-${skiaKeyFeatures}";
          hash = "sha256-gtoiVv0M6UGVJ/cspOmFihPBTSMHPaHmGNxZY5f2VWA=";
        };
      };
      # The URL template skia-bindings expands; {tag} and {key} are its own
      # placeholders, not Nix's.
      skiaBinariesUrl =
        pkgs:
        let
          archive = skiaArchives.${pkgs.stdenv.hostPlatform.system};
          name = "skia-binaries-${archive.key}.tar.gz";
          dir = pkgs.linkFarm "skia-binaries" [
            {
              inherit name;
              path = pkgs.fetchurl {
                url = "https://github.com/rust-skia/skia-binaries/releases/download/${skiaVersion}/${name}";
                inherit (archive) hash;
              };
            }
          ];
        in
        "file://${dir}/skia-binaries-{key}.tar.gz";

      # Libraries the binary opens at run time rather than links: the Vulkan
      # loader for wgpu, and the windowing libraries winit dlopens.
      runtimeLibs =
        pkgs: with pkgs; [
          vulkan-loader
          libGL
          wayland
          libxkbcommon
          libx11
          libxcursor
          libxi
          libxrandr
        ];

      nativeInputs =
        pkgs: with pkgs; [
          pkg-config
          cmake
          rustPlatform.bindgenHook
        ];

      buildInputs =
        pkgs: with pkgs; [
          # 8, by name: the engine needs 7 or newer, and nixpkgs' unversioned
          # `ffmpeg` is whichever major the distribution defaults to.
          ffmpeg_8
          # The cutout models' runtime; see ORT_LIB_LOCATION above.
          onnxruntime
          alsa-lib
          fontconfig
          freetype
          gtk3
          libxkbcommon
          wayland
          openssl
        ];
    in
    {
      packages = eachSystem (pkgs: rec {
        default = concat;
        concat = pkgs.rustPlatform.buildRustPackage {
          pname = "concat";
          version = "0.2.4";
          src = self;

          cargoRoot = "src";
          buildAndTestSubdir = "src";
          cargoLock.lockFile = ./src/Cargo.lock;
          cargoBuildFlags = [
            "-p"
            "concat"
          ];

          # The workspace's tests generate their own media through the
          # linked encoder and run anywhere the engine builds, but they take
          # minutes; `cargo test` in `nix develop` is where they belong.
          doCheck = false;

          env.SHERPA_ONNX_ARCHIVE_DIR = sherpaArchiveDir pkgs;
          env.SKIA_BINARIES_URL = skiaBinariesUrl pkgs;
          env.ORT_LIB_LOCATION = "${pkgs.onnxruntime}/lib";
          env.ORT_PREFER_DYNAMIC_LINK = "1";

          nativeBuildInputs = (nativeInputs pkgs) ++ [
            pkgs.wrapGAppsHook3
            pkgs.copyDesktopItems
          ];
          buildInputs = buildInputs pkgs;

          preFixup = ''
            gappsWrapperArgs+=(
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}
            )
          '';

          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "concat";
              exec = "concat";
              icon = "concat";
              desktopName = "Concat";
              comment = "Free and open source video editor";
              categories = [
                "AudioVideo"
                "AudioVideoEditing"
              ];
            })
          ];

          postInstall = ''
            install -Dm644 assets/concat_logo_512.png \
              $out/share/icons/hicolor/512x512/apps/concat.png
          '';

          meta = {
            description = "Free and open source video editor";
            homepage = "https://github.com/jub0t/Concat";
            license = pkgs.lib.licenses.agpl3Plus;
            platforms = systems;
            mainProgram = "concat";
          };
        };
      });

      devShells = eachSystem (pkgs: {
        default = pkgs.mkShell {
          packages =
            (nativeInputs pkgs)
            ++ (buildInputs pkgs)
            ++ (with pkgs; [
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer
            ]);

          env.SHERPA_ONNX_ARCHIVE_DIR = sherpaArchiveDir pkgs;
          env.SKIA_BINARIES_URL = skiaBinariesUrl pkgs;
          env.ORT_LIB_LOCATION = "${pkgs.onnxruntime}/lib";
          env.ORT_PREFER_DYNAMIC_LINK = "1";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);

          shellHook = ''
            echo "Concat: cd src && cargo run -p concat"
          '';
        };
      });

      formatter = eachSystem (pkgs: pkgs.nixfmt-rfc-style);
    };
}
