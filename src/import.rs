//! Les convertisseurs : un fichier d'un autre format devient un morceau
//! LilyPond du recueil.
//!
//! LilyPond fournit `musicxml2ly`, `midi2ly` et `abc2ly`. Le reste (les
//! fichiers de MuseScore, Guitar Pro, Capella, Power Tab…) passe d'abord par
//! MuseScore, qui l'exporte en MusicXML, s'il est installé.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::atelier::{self, Atelier};
use crate::recueil::{self, Recueil};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Un morceau LilyPond venu d'ailleurs : simplement copié.
    LilyPond,
    MusicXml,
    Midi,
    Abc,
    /// Tout ce que MuseScore sait ouvrir.
    MuseScore,
}

const MUSESCORE: [&str; 5] = ["mscore", "musescore", "mscore4portable", "musescore4", "mscore3"];

impl Format {
    /// Les extensions reconnues, par format.
    pub const EXTENSIONS: [(Format, &'static [&'static str]); 5] = [
        (Format::LilyPond, &["ly"]),
        (Format::MusicXml, &["musicxml", "mxl", "xml"]),
        (Format::Midi, &["mid", "midi", "kar"]),
        (Format::Abc, &["abc"]),
        (Format::MuseScore, &["mscz", "mscx", "gp", "gp3", "gp4", "gp5", "gpx", "ptb", "cap", "capx", "bww", "ove"]),
    ];

    pub fn detecter(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        Self::EXTENSIONS
            .iter()
            .find(|(_, exts)| exts.contains(&ext.as_str()))
            .map(|(f, _)| *f)
    }

    pub fn nom(self) -> &'static str {
        match self {
            Format::LilyPond => "LilyPond",
            Format::MusicXml => "MusicXML",
            Format::Midi => "MIDI",
            Format::Abc => "ABC",
            Format::MuseScore => "MuseScore",
        }
    }
}

/// Convertit `source` et l'ajoute au recueil ; rend le nom du nouveau morceau.
pub async fn importer(atelier: &Atelier, recueil: &Recueil, source: &Path) -> Result<String> {
    let format = Format::detecter(source).context("format inconnu")?;
    let etabli = atelier.etabli()?;
    let dir = etabli.path();
    let ly = dir.join("import.ly");

    match format {
        Format::LilyPond => std::fs::copy(source, &ly).map(drop).with_context(|| format!("lecture de {}", source.display()))?,
        Format::MusicXml => convertir("musicxml2ly", source, &ly, dir).await?,
        Format::Midi => convertir("midi2ly", source, &ly, dir).await?,
        Format::Abc => convertir("abc2ly", source, &ly, dir).await?,
        Format::MuseScore => {
            let Some(musescore) = atelier::trouver(&MUSESCORE) else {
                bail!("MuseScore est nécessaire pour ouvrir ce fichier (paquet « complet » de Vinteuil)");
            };
            let xml = dir.join("import.musicxml");
            let args: [&OsStr; 4] = [musescore.as_ref(), "-o".as_ref(), xml.as_ref(), source.as_ref()];
            // Sans écran : MuseScore n'ouvre aucune fenêtre.
            let sortie = atelier::lancer(&args, dir, &[("QT_QPA_PLATFORM", "offscreen")]).await?;
            if !sortie.reussi || !xml.exists() {
                bail!("MuseScore : {}", atelier::derniere_ligne(&sortie.stderr));
            }
            convertir("musicxml2ly", &xml, &ly, dir).await?;
        }
    }

    let texte = recueil::read(&ly)?;
    let base = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let destination = recueil.nom_libre(&base);
    recueil::write_atomic(&destination, &texte)?;
    Ok(recueil::nom(&destination))
}

async fn convertir(programme: &str, source: &Path, sortie: &PathBuf, dir: &Path) -> Result<()> {
    let args: [&OsStr; 4] = [programme.as_ref(), "-o".as_ref(), sortie.as_ref(), source.as_ref()];
    let resultat = atelier::lancer(&args, dir, &[]).await?;
    if !resultat.reussi || !sortie.exists() {
        bail!("{programme} : {}", atelier::derniere_ligne(&resultat.stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::glib;

    #[test]
    fn detection_par_extension() {
        assert_eq!(Format::detecter(Path::new("a/Sonate.MXL")), Some(Format::MusicXml));
        assert_eq!(Format::detecter(Path::new("air.mid")), Some(Format::Midi));
        assert_eq!(Format::detecter(Path::new("reel.abc")), Some(Format::Abc));
        assert_eq!(Format::detecter(Path::new("riff.gp5")), Some(Format::MuseScore));
        assert_eq!(Format::detecter(Path::new("morceau.ly")), Some(Format::LilyPond));
        assert_eq!(Format::detecter(Path::new("sans_extension")), None);
    }

    #[test]
    fn importer_abc_et_musicxml() {
        if atelier::trouver(&["abc2ly"]).is_none() || atelier::trouver(&["musicxml2ly"]).is_none() {
            eprintln!("convertisseurs de LilyPond absents : test sauté");
            return;
        }
        let atelier = Atelier::new().unwrap();
        let sources = atelier.etabli().unwrap();
        let racine = std::env::temp_dir().join(format!("vinteuil-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&racine);
        let recueil = Recueil::open_or_init(&racine).unwrap();

        let abc = sources.path().join("Reel.abc");
        std::fs::write(&abc, "X:1\nT:Reel\nM:4/4\nL:1/8\nK:D\nDEFG ABcd|]\n").unwrap();
        let xml = sources.path().join("Air.musicxml");
        std::fs::write(
            &xml,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<score-partwise version="4.0"><work><work-title>Air</work-title></work>
<part-list><score-part id="P1"><part-name>Piano</part-name></score-part></part-list>
<part id="P1"><measure number="1"><attributes><divisions>1</divisions>
<time><beats>4</beats><beat-type>4</beat-type></time><clef><sign>G</sign><line>2</line></clef></attributes>
<note><pitch><step>C</step><octave>5</octave></pitch><duration>4</duration><type>whole</type></note>
</measure></part></score-partwise>"#,
        )
        .unwrap();

        let ctx = glib::MainContext::new();
        assert_eq!(ctx.block_on(importer(&atelier, &recueil, &abc)).unwrap(), "Reel.ly");
        // Un second import du même nom ne remplace pas le premier.
        assert_eq!(ctx.block_on(importer(&atelier, &recueil, &abc)).unwrap(), "Reel 2.ly");
        assert_eq!(ctx.block_on(importer(&atelier, &recueil, &xml)).unwrap(), "Air.ly");
        let air = recueil::read(&recueil.chemin("Air.ly")).unwrap();
        assert_eq!(recueil::titre(&air).as_deref(), Some("Air"));

        let faux = sources.path().join("Faux.abc");
        std::fs::write(&faux, "pas de l'ABC du tout").unwrap();
        let _ = ctx.block_on(importer(&atelier, &recueil, &faux));
        std::fs::remove_dir_all(racine).unwrap();
    }
}
