# Module Home Manager. Dans /etc/nixos :
#   inputs.vinteuil.url = "path:/home/aristide/code/music";
#   imports = [ inputs.vinteuil.homeManagerModules.default ];
#   programs.vinteuil.enable = true;
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.vinteuil;
  toml = pkgs.formats.toml { };
  paquets = self.packages.${pkgs.stdenv.hostPlatform.system};
in
{
  options.programs.vinteuil = {
    enable = lib.mkEnableOption "Vinteuil, le pupitre zen pour LilyPond";

    museScore = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Importer aussi via MuseScore (.mscz, Guitar Pro, Capella…).";
    };

    package = lib.mkOption {
      type = lib.types.package;
      default = if cfg.museScore then paquets.complet else paquets.default;
      description = "Le paquet Vinteuil.";
    };

    fonts = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = with pkgs; [ literata eb-garamond ];
      description = "Polices des titres et des paroles.";
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = { };
      example = {
        police = "EB Garamond";
        largeur = 170;
        portee = 20;
        theme = "papier";
        plein_ecran = true;
      };
      description = "Écrit dans ~/.config/vinteuil/config.toml.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ] ++ cfg.fonts;
    fonts.fontconfig.enable = lib.mkIf (cfg.fonts != [ ]) (lib.mkDefault true);
    xdg.configFile."vinteuil/config.toml" = lib.mkIf (cfg.settings != { }) {
      source = toml.generate "vinteuil-config.toml" cfg.settings;
    };
  };
}
