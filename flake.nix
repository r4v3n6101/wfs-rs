{
  description = "FUSE filesystem for displaying Half-Life's WAD-s.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustVersion = pkgs.rust-bin.nightly.latest.default;
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustVersion;
          rustc = rustVersion;
        };
        manifest = (pkgs.lib.importTOML ./Cargo.toml).package;

        buildInputs =
          if pkgs.stdenv.isLinux then
            [ pkgs.fuse3 ]
          else if pkgs.stdenv.isDarwin then
            [ pkgs.macfuse-stubs ]
          else
            [ ];
        nativeBuildInputs = [ pkgs.pkg-config ];

        rustBuiltPackage = rustPlatform.buildRustPackage {
          inherit buildInputs nativeBuildInputs;
          pname = manifest.name;
          version = manifest.version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };
      in
      {
        formatter = pkgs.nixfmt-rfc-style;
        packages.default = rustBuiltPackage;
        devShell = pkgs.mkShell rec {
          inputsFrom = [ rustBuiltPackage ];

          packages = [
            (rustVersion.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
            })
          ];

          RUST_BACKTRACE = 1;
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath buildInputs}";
        };
      }
    );
}
