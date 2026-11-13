{
  description = "Squeak — open-source HID++ 2.0 peripheral configurator, daemon, and CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "squeak";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            wrapGAppsHook4
          ];

          buildInputs = with pkgs; [
            gtk4
            libadwaita
            glib
          ];

          postInstall = ''
            install -Dm644 data/applications/dev.squeak.Squeak.desktop $out/share/applications/dev.squeak.Squeak.desktop
            install -Dm644 data/metainfo/dev.squeak.Squeak.metainfo.xml $out/share/metainfo/dev.squeak.Squeak.metainfo.xml
            install -Dm644 data/icons/hicolor/scalable/apps/dev.squeak.Squeak.svg $out/share/icons/hicolor/scalable/apps/dev.squeak.Squeak.svg
            install -Dm644 data/icons/hicolor/128x128/apps/dev.squeak.Squeak.png $out/share/icons/hicolor/128x128/apps/dev.squeak.Squeak.png
            install -Dm644 data/icons/hicolor/256x256/apps/dev.squeak.Squeak.png $out/share/icons/hicolor/256x256/apps/dev.squeak.Squeak.png
            install -Dm644 data/icons/hicolor/512x512/apps/dev.squeak.Squeak.png $out/share/icons/hicolor/512x512/apps/dev.squeak.Squeak.png
            install -Dm644 data/dbus/dev.squeak.Daemon.service $out/share/dbus-1/services/dev.squeak.Daemon.service
            install -Dm644 data/systemd/squeakd.service $out/lib/systemd/user/squeakd.service
          '';

          meta = with pkgs.lib; {
            description = "Open-source HID++ 2.0 peripheral configurator, daemon, and CLI";
            homepage = "https://github.com/0x70776c/Squeak";
            license = licenses.gpl3Plus;
            platforms = platforms.linux;
            mainProgram = "squeak-gui";
          };
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            pkg-config
          ];
          buildInputs = with pkgs; [
            gtk4
            libadwaita
            glib
          ];
        };
      });
}
