{
  description = "Vinteuil — un pupitre zen pour les partitions LilyPond";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        default = pkgs.callPackage ./nix/package.nix { craneLib = crane.mkLib pkgs; };
        # Avec MuseScore pour importer .mscz, Guitar Pro, Capella… (lourd).
        complet = default.override { avecMuseScore = true; };
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
      });

      devShells = forAll (pkgs: {
        default = (crane.mkLib pkgs).devShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = with pkgs; [ rust-analyzer clippy rustfmt lilypond ];
        };
      });

      checks = forAll (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      });

      homeManagerModules.default = import ./nix/hm-module.nix self;
    };
}
