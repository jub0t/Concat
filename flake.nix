{
  # Nix support (#7). Packages the deprecated Tauri app in desktop/, which no
  # longer builds against the engine (the engine links FFmpeg now and the
  # host's Rust moved into engine/crates). To be repointed at the Slint
  # window, engine/crates/concat, once that is packaged.
  #
  # Linux only on purpose: Tauri under Nix on macOS means
  # Apple SDK juggling, and the people asking for a flake run Linux.
  #
  #   nix build .#concat     the app, with ffmpeg and whisper-cli wired in
  #   nix develop             a shell with everything `npm run app` needs
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
    in
    {
      packages = eachSystem (pkgs: rec {
        default = concat;
        concat = pkgs.rustPlatform.buildRustPackage rec {
          pname = "concat";
          version = "0.2.0";
          src = self;

          # No vendor hashes to keep in sync: cargo dependencies come straight
          # from the Cargo.lock, npm dependencies straight from the integrity
          # fields of package-lock.json.
          cargoRoot = "desktop/src-tauri";
          buildAndTestSubdir = cargoRoot;
          cargoLock.lockFile = ./desktop/src-tauri/Cargo.lock;

          npmRoot = "desktop";
          npmDeps = pkgs.importNpmLock { npmRoot = ./desktop; };

          # The release guard in build.rs insists on staged ffmpeg/whisper
          # binaries because a Finder/desktop launch has no PATH to fall back
          # on. The Nix wrapper guarantees them on PATH, so this build opts
          # out of bundling explicitly.
          env.CONCAT_SYSTEM_TOOLS = "1";

          nativeBuildInputs = with pkgs; [
            cargo-tauri.hook
            nodejs
            importNpmLock.npmConfigHook
            pkg-config
            wrapGAppsHook3
            copyDesktopItems
          ];

          buildInputs = with pkgs; [
            openssl
            webkitgtk_4_1
            gtk3
            libsoup_3
            # TLS for the webview and for whisper model downloads.
            glib-networking
            # cpal's ALSA backend, for engine audio playback.
            alsa-lib
          ];

          # The app looks for ffmpeg/ffprobe and whisper-cli beside itself
          # first and falls back to PATH (engine `binaries.rs`, desktop
          # `transcribe.rs`), so the Nix build bundles nothing and puts
          # nixpkgs' copies on the wrapper's PATH instead. The library path
          # is for the wgpu compositor, which loads Vulkan (or GL) at run
          # time rather than linking it.
          preFixup = ''
            gappsWrapperArgs+=(
              --prefix PATH : ${
                pkgs.lib.makeBinPath [
                  pkgs.ffmpeg
                  pkgs.whisper-cpp
                ]
              }
              --prefix LD_LIBRARY_PATH : ${
                pkgs.lib.makeLibraryPath [
                  pkgs.vulkan-loader
                  pkgs.libGL
                ]
              }
            )
          '';

          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "concat";
              exec = "concat-desktop";
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
            install -Dm644 desktop/src-tauri/icons/128x128.png \
              $out/share/icons/hicolor/128x128/apps/concat.png
          '';

          meta = {
            description = "Free and open source video editor";
            homepage = "https://github.com/jub0t/Concat";
            license = pkgs.lib.licenses.agpl3Plus;
            platforms = systems;
            mainProgram = "concat-desktop";
          };
        };
      });

      devShells = eachSystem (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.system}.concat ];
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            cargo-tauri
            ffmpeg
            whisper-cpp
          ];
          # Same runtime loads as the wrapped app, for `npm run app`.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.vulkan-loader
            pkgs.libGL
          ];
        };
      });
    };
}
