{
  description = "Rust environment example";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix/monthly";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { fenix, flake-utils, nixpkgs, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-zm3dyIY2T414ZRR3EhLOvptzG6gta4WZUcawzMUWtqI=";
        };
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        formatter = pkgs.nixpkgs-fmt;
        devShells.default = pkgs.stdenv.mkDerivation {
          name = "rust environment";
          nativeBuildInputs = [
            toolchain
            pkgs.z3
            pkgs.nodejs
            # editors/tree-sitter-hibana の parser.c を作る（tree-sitter generate）
            pkgs.tree-sitter
          ];
          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          # build.rs が実行ファイルの rpath に入れる（Z3 のライブラリを FFI で使う）
          Z3_LIB_DIR = "${pkgs.z3.lib}/lib";
        };
      }
    );
}
