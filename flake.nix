{
  # Concat on Nix, Linux only: the people asking for a flake run Linux, and
  # the macOS and Windows builds ship from release.yml.
  #
  #   nix build            the editor window, ./result/bin/concat
  #   nix run              build and launch it
  #   nix develop          a shell with everything `cargo run -p concat` needs
  #
  # The build is one Cargo workspace under engine/, so cargo dependencies
  # come straight from engine/Cargo.lock and there is no vendor hash to keep
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
  #
  # The window is built with the FemtoVG-over-wgpu renderer (`--features
  # wgpu`) rather than Skia: skia-bindings also downloads its binaries at
  # build time, and the wgpu renderer is pure Rust. Vulkan is loaded at run
  # time, hence the LD_LIBRARY_PATH on the wrapper.
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

      # Must match the sherpa-onnx-sys version in engine/Cargo.lock: the sys
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

      # Libraries the binary opens at run time rather than links: the Vulkan
      # loader for wgpu, and the windowing libraries winit dlopens.
      runtimeLibs =
        pkgs: with pkgs; [
          vulkan-loader
          libGL
          wayland
          libxkbcommon
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
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
          alsa-lib
          fontconfig
          freetype
          gtk3
          libxkbcommon
          wayland
        ];
    in
    {
      packages = eachSystem (pkgs: rec {
        default = concat;
        concat = pkgs.rustPlatform.buildRustPackage {
          pname = "concat";
          version = "0.2.0";
          src = self;

          cargoRoot = "engine";
          buildAndTestSubdir = "engine";
          cargoLock.lockFile = ./engine/Cargo.lock;
          cargoBuildFlags = [
            "-p"
            "concat"
            "--no-default-features"
            "--features"
            "wgpu"
          ];

          # The workspace's tests generate their own media through the
          # linked encoder and run anywhere the engine builds, but they take
          # minutes; `cargo test` in `nix develop` is where they belong.
          doCheck = false;

          env.SHERPA_ONNX_ARCHIVE_DIR = sherpaArchiveDir pkgs;

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
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);

          shellHook = ''
            echo "Concat: cd engine && cargo run -p concat --no-default-features --features wgpu"
          '';
        };
      });

      formatter = eachSystem (pkgs: pkgs.nixfmt-rfc-style);
    };
}
