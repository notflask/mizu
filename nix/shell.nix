{ lib
, stdenv
, callPackage
, mkShell
, rustc
, cargo
, clippy
, rustfmt
, rust-analyzer
, pkg-config
, rustPlatform
, gnumake
, python3
}:

let
  runtimeLibs = callPackage ./runtime-libs.nix { };
in
mkShell {
  nativeBuildInputs = [
    rustc
    cargo
    clippy
    rustfmt
    rust-analyzer
    pkg-config
    gnumake
    python3
    rustPlatform.bindgenHook
  ];
  buildInputs = runtimeLibs;
  RUST_SRC_PATH = rustPlatform.rustLibSrc;
  LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
}
