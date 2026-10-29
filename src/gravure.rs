//! La gravure : LilyPond met le morceau en SVG, sur une seule page aussi
//! haute que la musique, à la largeur de la colonne.
//!
//! Le fichier du morceau n'est jamais touché : une enveloppe l'inclut, puis
//! impose sa mise en page. LilyPond ne grave la partition qu'à la fin du
//! fichier, si bien que ce `\paper`, venu après, l'emporte sur celui du
//! morceau (celui que `musicxml2ly` recopie de la page d'origine, par exemple).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::atelier::{self, Atelier};

pub struct Reglages {
    /// Largeur de la ligne de musique, en millimètres.
    pub largeur: f64,
    /// Taille de la portée, en points.
    pub portee: f64,
    /// Police des titres et des paroles.
    pub police: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erreur {
    /// La ligne fautive du morceau (à partir de 1), si LilyPond l'a nommée.
    pub ligne: Option<u32>,
    pub message: String,
}

impl Erreur {
    fn simple(message: impl Into<String>) -> Self {
        Self {
            ligne: None,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Erreur {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.ligne {
            Some(l) => write!(f, "Ligne {l} : {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Une chaîne Scheme / LilyPond entre guillemets.
fn chaine(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// De quoi loger ce qui déborde à gauche de la portée : numéros de mesure, accolades.
const MARGE: f64 = 6.0;

/// La largeur de la page gravée, marges comprises, en millimètres.
pub fn largeur_page(largeur: f64) -> f64 {
    largeur + 2.0 * MARGE
}

pub fn enveloppe(morceau: &Path, r: &Reglages) -> String {
    format!(
        r#"\include {fichier}
#(set-global-staff-size {portee})
\paper {{
  paper-width = {papier}\mm
  line-width = {ligne}\mm
  left-margin = {MARGE}\mm
  top-margin = 6\mm
  bottom-margin = 6\mm
  page-breaking = #ly:one-page-breaking
  % Sans quoi un morceau qui demande d'étirer la musique jusqu'au bas de
  % la page la rend démesurée.
  ragged-bottom = ##t
  ragged-last-bottom = ##t
  print-page-number = ##f
  oddHeaderMarkup = ##f
  evenHeaderMarkup = ##f
  oddFooterMarkup = ##f
  evenFooterMarkup = ##f
  property-defaults.fonts.serif = {police}
}}
"#,
        fichier = chaine(&morceau.to_string_lossy()),
        portee = r.portee,
        papier = largeur_page(r.largeur),
        ligne = r.largeur,
        police = chaine(&r.police),
    )
}

/// Un lien de LilyPond : l'endroit d'un objet sur la page (une note, une
/// annotation…) et sa position dans le source.
#[derive(Debug, Clone, PartialEq)]
pub struct Lien {
    /// x1, y1, x2, y2, en fractions de la largeur de la page, y vers le bas.
    pub cadre: [f64; 4],
    /// À partir de 0.
    pub ligne: usize,
    /// En caractères, à partir de 0.
    pub colonne: usize,
}

/// Une page gravée : l'image, et où sont les notes du morceau.
#[derive(Debug)]
pub struct Page {
    pub svg: String,
    pub liens: Vec<Lien>,
}

/// Grave le morceau ; rend une page par livre (le plus souvent une seule).
pub async fn graver(atelier: &Atelier, morceau: &Path, r: &Reglages) -> Result<Vec<Page>, Erreur> {
    let etabli = atelier
        .etabli()
        .map_err(|e| Erreur::simple(format!("{e:#}")))?;
    let dir = etabli.path();
    let enveloppe_path = dir.join("partition.ly");
    std::fs::write(&enveloppe_path, enveloppe(morceau, r))
        .map_err(|e| Erreur::simple(e.to_string()))?;

    let mut include = std::ffi::OsString::from("--include=");
    include.push(morceau.parent().unwrap_or(Path::new(".")));
    // Le moteur Cairo : dix fois plus rapide que le SVG maison de LilyPond
    // (1 s au lieu de 13 pour une page de violon seul). Le même passage écrit
    // aussi un PDF : le SVG de Cairo n'a pas les liens vers le source, le PDF si.
    let args: [&OsStr; 9] = [
        "lilypond".as_ref(),
        "--svg".as_ref(),
        "--pdf".as_ref(),
        "-dbackend=cairo".as_ref(),
        "-dpoint-and-click".as_ref(),
        "-drelative-includes".as_ref(),
        "--loglevel=WARNING".as_ref(),
        &include,
        "partition.ly".as_ref(),
    ];
    let sortie = atelier::lancer(&args, dir, &[]).await.map_err(|e| {
        Erreur::simple(format!(
            "{e:#} : LilyPond est nécessaire pour graver la partition"
        ))
    })?;
    if !sortie.reussi {
        return Err(erreur(&sortie.stderr, morceau));
    }

    let mut pages: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| Erreur::simple(e.to_string()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "svg"))
        .collect();
    // `partition.svg`, ou `partition-1.svg`, `partition-2.svg`… pour plusieurs livres.
    pages.sort_by_key(|p| {
        p.file_stem()
            .and_then(|s| s.to_str()?.rsplit_once('-')?.1.parse::<u32>().ok())
            .unwrap_or(0)
    });
    if pages.is_empty() {
        return Err(Erreur::simple(
            "Rien à graver : le morceau ne contient pas de musique",
        ));
    }
    pages
        .iter()
        .map(|p| {
            let svg = std::fs::read_to_string(p)
                .map(encre_variable)
                .map_err(|e| Erreur::simple(e.to_string()))?;
            let liens = std::fs::read(p.with_extension("pdf"))
                .map(|pdf| liens(&pdf, morceau))
                .unwrap_or_default();
            Ok(Page { svg, liens })
        })
        .collect()
}

/// Le noir tel que l'écrit le moteur Cairo de LilyPond.
const NOIR: &str = "rgb(0%, 0%, 0%)";

/// Le noir devient `currentColor` : la partition prendra la couleur du thème.
/// Les couleurs voulues par le morceau (`\with-color`) restent les siennes.
fn encre_variable(svg: String) -> String {
    svg.replace(NOIR, "currentColor")
}

/// Les liens du PDF de Cairo qui mènent au morceau. Chacun est une annotation
/// PDF en clair : `/Rect [ x1 y1 x2 y2 ]` puis `/URI (textedit://chemin:ligne:car:col)`,
/// en points, l'origine en bas à gauche de la page (`/MediaBox`).
pub fn liens(pdf: &[u8], morceau: &Path) -> Vec<Lien> {
    let texte = String::from_utf8_lossy(pdf);
    let nombres = |apres: &str| -> Option<[f64; 4]> {
        let fin = apres.find(']')?;
        let v: Vec<f64> = apres[..fin]
            .split_whitespace()
            .filter_map(|n| n.parse().ok())
            .collect();
        v.try_into().ok()
    };
    let Some([_, _, largeur, hauteur]) = texte
        .split_once("/MediaBox [")
        .and_then(|(_, r)| nombres(r))
    else {
        return Vec::new();
    };
    let fichier = morceau.to_string_lossy();
    let mut liens = Vec::new();
    let morceaux = texte.split("/Rect [").skip(1);
    for bloc in morceaux {
        let (Some([x1, y1, x2, y2]), Some(uri)) = (
            nombres(bloc),
            bloc.split_once("/URI (").map(|(_, r)| chaine_pdf(r)),
        ) else {
            continue;
        };
        let Some(cible) = uri.strip_prefix("textedit://") else {
            continue;
        };
        // chemin:ligne:caractère:colonne ; le chemin peut contenir des « : ».
        let mut parties = cible.rsplitn(4, ':');
        let (_, car, ligne, chemin) = (
            parties.next(),
            parties.next(),
            parties.next(),
            parties.next(),
        );
        let (Some(car), Some(ligne), Some(chemin)) = (car, ligne, chemin) else {
            continue;
        };
        let ici = chemin == fichier || Path::new(chemin).file_name() == morceau.file_name();
        let (Ok(colonne), Ok(ligne)) = (car.parse::<usize>(), ligne.parse::<usize>()) else {
            continue;
        };
        if !ici || ligne == 0 {
            continue;
        }
        let (bas, haut) = (y1.min(y2), y1.max(y2));
        liens.push(Lien {
            cadre: [
                x1.min(x2) / largeur,
                (hauteur - haut) / largeur,
                x1.max(x2) / largeur,
                (hauteur - bas) / largeur,
            ],
            ligne: ligne - 1,
            colonne,
        });
    }
    liens
}

/// Une chaîne PDF `(…)`, déjà ouverte : jusqu'à la parenthèse fermante.
fn chaine_pdf(s: &str) -> String {
    let mut sortie = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            ')' => break,
            '\\' => sortie.extend(chars.next()),
            c => sortie.push(c),
        }
    }
    sortie
}

/// La première erreur que LilyPond signale, avec sa ligne si elle est dans le
/// morceau (`/chemin/morceau.ly:12:5: error: …`).
pub fn erreur(stderr: &str, morceau: &Path) -> Erreur {
    let fichier = morceau.to_string_lossy();
    for ligne in stderr.lines() {
        let Some((lieu, message)) = ligne
            .split_once(": error: ")
            .or_else(|| ligne.split_once(": fatal error: "))
        else {
            continue;
        };
        let mut parties = lieu.rsplitn(3, ':');
        let (_colonne, numero, chemin) = (parties.next(), parties.next(), parties.next());
        let numero = numero.and_then(|n| n.parse().ok());
        let ici =
            chemin.is_some_and(|c| c == fichier || Path::new(c).file_name() == morceau.file_name());
        return Erreur {
            ligne: numero.filter(|_| ici),
            message: message.trim().to_string(),
        };
    }
    Erreur::simple(atelier::derniere_ligne(stderr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reglages() -> Reglages {
        Reglages {
            largeur: 150.0,
            portee: 20.0,
            police: "Literata".into(),
        }
    }

    #[test]
    fn enveloppe_echappe_le_chemin() {
        let e = enveloppe(Path::new("/p/Le \"bon\" air.ly"), &reglages());
        assert!(e.starts_with("\\include \"/p/Le \\\"bon\\\" air.ly\"\n"));
        assert!(e.contains("paper-width = 162\\mm"));
        assert!(e.contains("line-width = 150\\mm"));
        assert!(e.contains("property-defaults.fonts.serif = \"Literata\""));
        assert!(e.contains("ragged-bottom = ##t"));
    }

    #[test]
    fn liens_du_pdf() {
        let pdf = b"<< /Type /Page /MediaBox [ 0 0 400 800 ] >>\n\
            << /Rect [ 100 700 110 708 ]\n /A << /URI (textedit:///r/Air \\(1\\).ly:3:13:13) >> >>\n\
            << /Rect [ 10 10 20 20 ]\n /A << /URI (textedit:///usr/share/lilypond/init.ly:5:1:1) >> >>\n";
        let l = liens(pdf, Path::new("/r/Air (1).ly"));
        // Le lien vers un autre fichier est ignoré.
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].ligne, l[0].colonne), (2, 13));
        assert_eq!(l[0].cadre, [0.25, 0.23, 0.275, 0.25]);
        assert!(liens(b"pas un pdf", Path::new("/r/Air.ly")).is_empty());
    }

    #[test]
    fn seul_le_noir_change() {
        let svg = r#"<path fill="rgb(0%, 0%, 0%)"/><path stroke="rgb(0%, 0%, 0%)" fill="rgb(100%, 0%, 0%)"/>"#;
        assert_eq!(
            encre_variable(svg.into()),
            r#"<path fill="currentColor"/><path stroke="currentColor" fill="rgb(100%, 0%, 0%)"/>"#
        );
    }

    #[test]
    fn erreur_avec_ligne() {
        let m = Path::new("/r/Air.ly");
        let e = erreur(
            "Processing `partition.ly'\n/r/Air.ly:7:12: error: not a note name: cx\n  c4 cx\n",
            m,
        );
        assert_eq!(
            e,
            Erreur {
                ligne: Some(7),
                message: "not a note name: cx".into()
            }
        );
        // Une erreur dans l'enveloppe n'a pas de ligne dans le morceau.
        let e = erreur("partition.ly:3:1: error: oups", m);
        assert_eq!(e.ligne, None);
        assert_eq!(
            erreur("\n\nquelque chose a mal tourné\n", m).message,
            "quelque chose a mal tourné"
        );
        assert_eq!(e.to_string(), "oups");
    }

    #[test]
    fn graver_un_morceau() {
        if atelier::trouver(&["lilypond"]).is_none() {
            eprintln!("lilypond absent : test sauté");
            return;
        }
        let atelier = Atelier::new().unwrap();
        let etabli = atelier.etabli().unwrap();
        let morceau = etabli.path().join("Essai.ly");
        std::fs::write(
            &morceau,
            "\\header { title = \"Essai\" }\n\\relative c' { c4 d e f | g1 }\n",
        )
        .unwrap();
        let pages = glib::MainContext::new()
            .block_on(graver(&atelier, &morceau, &reglages()))
            .unwrap();
        assert_eq!(pages.len(), 1);
        // 162 mm, en points (arrondis au-dessus par Cairo).
        let svg = &pages[0].svg;
        assert!(svg.contains("width=\"460\""), "{}", &svg[..300]);
        assert!(svg.contains("currentColor"));
        assert!(!svg.contains(NOIR));
        // Les cinq notes de la ligne 2, dans l'ordre de la page.
        let mut liens = pages[0].liens.clone();
        liens.sort_by(|a, b| a.cadre[0].total_cmp(&b.cadre[0]));
        let colonnes: Vec<_> = liens.iter().map(|l| (l.ligne, l.colonne)).collect();
        assert_eq!(colonnes, [(1, 15), (1, 18), (1, 20), (1, 22), (1, 26)]);
        assert!(liens
            .iter()
            .all(|l| l.cadre[0] < l.cadre[2] && l.cadre[1] < l.cadre[3]));

        std::fs::write(&morceau, "\\relative c' {\n  c4 d\n  cx4\n}\n").unwrap();
        let e = glib::MainContext::new()
            .block_on(graver(&atelier, &morceau, &reglages()))
            .unwrap_err();
        assert_eq!(e.ligne, Some(3), "{e}");
    }

    use gtk::glib;
}
