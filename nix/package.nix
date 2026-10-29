{ lib
, craneLib
, pkg-config
, wrapGAppsHook4
, copyDesktopItems
, makeDesktopItem
, gtk4
, libadwaita
, librsvg
, lilypond
, musescore
, literata
, eb-garamond
, writeText
  # MuseScore ouvre ses propres fichiers, Guitar Pro, Capella, Power Tab… :
  # des centaines de Mo, donc seulement sur demande.
, avecMuseScore ? false
}:

let
  common = {
    src = craneLib.cleanCargoSource ../.;
    strictDeps = true;
    nativeBuildInputs = [ pkg-config ];
    buildInputs = [ gtk4 libadwaita librsvg ];
  };
  cargoArtifacts = craneLib.buildDepsOnly common;

  # La configuration fontconfig du système, plus les polices de la partition.
  fontsConf = writeText "vinteuil-fonts.conf" ''
    <?xml version="1.0"?>
    <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
    <fontconfig>
      <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
      <dir>${literata}/share/fonts</dir>
      <dir>${eb-garamond}/share/fonts</dir>
    </fontconfig>
  '';

  # LilyPond grave, et fournit musicxml2ly, midi2ly et abc2ly.
  outils = [ lilypond ] ++ lib.optional avecMuseScore musescore;
in
craneLib.buildPackage (common // {
  inherit cargoArtifacts;
  nativeBuildInputs = common.nativeBuildInputs ++ [ wrapGAppsHook4 copyDesktopItems ];
  # Les tests qui gravent ou convertissent s'en servent.
  nativeCheckInputs = [ lilypond ];

  preFixup = ''
    gappsWrapperArgs+=(--set-default FONTCONFIG_FILE ${fontsConf})
    gappsWrapperArgs+=(--prefix PATH : ${lib.makeBinPath outils})
  '';

  desktopItems = [
    (makeDesktopItem {
      name = "vinteuil";
      desktopName = "Vinteuil";
      genericName = "Partitions";
      comment = "Un pupitre zen pour les partitions LilyPond";
      exec = "vinteuil %f";
      icon = "audio-x-generic";
      categories = [ "AudioVideo" "Audio" ];
      mimeTypes = [
        "text/x-lilypond"
        "application/vnd.recordare.musicxml+xml"
        "application/vnd.recordare.musicxml"
        "audio/midi"
        "text/vnd.abc"
      ];
      startupWMClass = "vinteuil";
    })
  ];

  meta = {
    description = "Un pupitre zen pour les partitions LilyPond";
    license = lib.licenses.gpl3Plus;
    mainProgram = "vinteuil";
    platforms = lib.platforms.linux;
  };
})
