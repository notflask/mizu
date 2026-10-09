{ lib
, stdenv
, callPackage
, rustPlatform
, pkg-config
, makeWrapper
}:

let
  runtimeLibs = callPackage ./runtime-libs.nix { };
  manifest = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "mizu";
  version = manifest.package.version;

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: type:
      let name = baseNameOf path; in
      !(name == "target" || name == ".git" || name == "result");
  };

  cargoLock.lockFile = ../Cargo.lock;

  # MuPDF is compiled from the sources vendored in the `mupdf-sys` crate;
  # bindgen needs libclang.
  nativeBuildInputs = [ pkg-config makeWrapper rustPlatform.bindgenHook ];
  buildInputs = runtimeLibs;

  # The tests need a GPU-free environment only; they do run, but keep the
  # build fast and deterministic.
  doCheck = false;

  postInstall = ''
    install -Dm644 packaging/linux/io.github.notflask.Mizu.desktop \
      $out/share/applications/io.github.notflask.Mizu.desktop
    if [ -d assets/icons/generated/linux/hicolor ]; then
      mkdir -p $out/share/icons
      cp -r assets/icons/generated/linux/hicolor $out/share/icons/
    fi
  '';

  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    wrapProgram $out/bin/mizu \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
  '';

  meta = {
    description = "A minimal, fast PDF viewer with Vim keybindings, a dark mode for the page itself, and ink";
    homepage = "https://github.com/notflask/mizu";
    license = lib.licenses.agpl3Plus;
    mainProgram = "mizu";
    platforms = lib.platforms.unix;
  };
}
