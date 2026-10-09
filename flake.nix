{
  description = "mizu — a minimal, fast PDF viewer with Vim keys, a dark mode for the page itself, and ink";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        mizu = pkgs.callPackage ./nix/package.nix { };
        default = mizu;
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.mizu}/bin/mizu";
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.callPackage ./nix/shell.nix { };
      });

      # For NixOS / home-manager: `nixpkgs.overlays = [ mizu.overlays.default ];`
      overlays.default = final: _prev: {
        mizu = final.callPackage ./nix/package.nix { };
      };
    };
}
