{
  description = "listello: a widget manager for the MacBook Pro Touch Bar, on Linux";
  inputs = { nixpkgs.url = "github:nixos/nixpkgs/nixos-24.11"; };
  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = forAllSystems (system: import nixpkgs { inherit system; });
    in rec {
      packages = forAllSystems (system:
        let pkgs = pkgsFor.${system};
        in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "listello";
            version = "0.1.0";
            src = ./.;
            cargoLock = { lockFile = ./Cargo.lock; };
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [
              pkgs.cairo
              pkgs.libinput
              pkgs.freetype
              pkgs.fontconfig
              pkgs.glib
              pkgs.pango
              pkgs.gdk-pixbuf
              pkgs.libxml2
              pkgs.librsvg
            ];

            postConfigure = ''
              substituteInPlace etc/systemd/system/listello-core.service \
                  --replace-fail /usr/bin $out/bin
              substituteInPlace crates/*/src/*.rs --replace-quiet /usr/share $out/share
            '';

            postInstall = ''
              cp -R etc $out/lib
              cp -R share $out
            '';
          };
        });

      devShells = forAllSystems (system:
        let pkgs = pkgsFor.${system};
        in {
          default = pkgs.mkShell {
            inputsFrom = [ packages.${system}.default ];
            packages = [ pkgs.rustfmt pkgs.rust-analyzer ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        });
    };
}
