//! Un recueil sur disque : un dossier, un fichier `.ly` par morceau. Rien
//! d'autre : le dossier se lit et se modifie aussi bien hors de l'application.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Un nouveau morceau, prêt à être complété.
const MODELE: &str = r#"\version "2.24.0"

\header {
  title = "Sans titre"
  composer = ""
}

\relative c' {
  \clef treble
  \time 4/4
  c4 d e f | g1 \bar "|."
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Morceau {
    /// Le nom du fichier, dans le dossier du recueil.
    pub fichier: String,
    /// Le titre de l'en-tête, ou à défaut le nom du fichier.
    pub titre: String,
}

pub struct Recueil {
    pub root: PathBuf,
}

impl Recueil {
    /// Ouvre le recueil, ou crée son dossier s'il n'existe pas.
    pub fn open_or_init(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).with_context(|| format!("création de {}", root.display()))?;
        Ok(Self { root: root.canonicalize()? })
    }

    pub fn titre(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Recueil".into())
    }

    pub fn chemin(&self, fichier: &str) -> PathBuf {
        self.root.join(fichier)
    }

    /// Les morceaux du dossier, par ordre alphabétique de fichier.
    pub fn morceaux(&self) -> Vec<Morceau> {
        let Ok(entrees) = fs::read_dir(&self.root) else { return Vec::new() };
        let mut morceaux: Vec<Morceau> = entrees
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let fichier = path.file_name()?.to_str()?.to_string();
                if fichier.starts_with('.') || path.extension()? != "ly" || !path.is_file() {
                    return None;
                }
                let stem = path.file_stem()?.to_string_lossy().into_owned();
                let titre = read(&path).ok().as_deref().and_then(titre).unwrap_or(stem);
                Some(Morceau { fichier, titre })
            })
            .collect();
        morceaux.sort_by_key(|m| m.fichier.to_lowercase());
        morceaux
    }

    /// Crée un morceau vierge ; rend son nom de fichier.
    pub fn nouveau(&self) -> Result<String> {
        let path = self.nom_libre("Sans titre");
        write_atomic(&path, MODELE)?;
        Ok(nom(&path))
    }

    /// `base.ly` dans le recueil, ou `base 2.ly`, `base 3.ly`… s'il est pris.
    pub fn nom_libre(&self, base: &str) -> PathBuf {
        let base = base.replace(['/', '\0'], "-");
        let base = if base.trim().is_empty() { "Sans titre".to_string() } else { base };
        (1..)
            .map(|n| match n {
                1 => self.root.join(format!("{base}.ly")),
                n => self.root.join(format!("{base} {n}.ly")),
            })
            .find(|p| !p.exists())
            .expect("une suite infinie trouve toujours un nom libre")
    }
}

pub fn nom(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Le titre de l'en-tête : le premier `title = "…"`.
pub fn titre(source: &str) -> Option<String> {
    source.match_indices("title").find_map(|(i, _)| {
        // Pas `subtitle`, ni `my-title`.
        if source[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            return None;
        }
        let reste = source[i + "title".len()..].trim_start().strip_prefix('=')?;
        let reste = reste.trim_start();
        let Some(reste) = reste.strip_prefix('"') else {
            // Un seul mot, sans guillemets (`title = Air`, comme l'écrit musicxml2ly).
            let mot: String = reste.chars().take_while(|c| c.is_alphanumeric() || *c == '-').collect();
            return (!mot.is_empty()).then_some(mot);
        };
        let mut titre = String::new();
        let mut chars = reste.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => titre.extend(chars.next()),
                c => titre.push(c),
            }
        }
        let titre = titre.trim().to_string();
        (!titre.is_empty()).then_some(titre)
    })
}

pub fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))
}

/// Écrit dans un fichier temporaire puis le renomme : une coupure en pleine
/// écriture ne laisse jamais un morceau à moitié écrit.
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().context("chemin sans dossier parent")?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().context("chemin sans nom")?.to_string_lossy()
    ));
    let mut f = fs::File::create(&tmp)?;
    f.write_all(content.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("écriture de {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vinteuil-{nom}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn titre_de_l_en_tete() {
        assert_eq!(titre("\\header {\n  title = \"Gymnopédie n° 1\"\n}"), Some("Gymnopédie n° 1".into()));
        assert_eq!(titre("title=\"Le \\\"Coucou\\\"\""), Some("Le \"Coucou\"".into()));
        // `subtitle` n'est pas le titre, ni un titre vide.
        assert_eq!(titre("subtitle = \"Andante\"\ntitle = \"\""), None);
        assert_eq!(titre("\\relative c' { c4 }"), None);
        assert_eq!(titre("\\header {\n  title = Air\n}"), Some("Air".into()));
        assert_eq!(titre("title = \\markup { Air }"), None);
    }

    #[test]
    fn morceaux_et_noms_libres() {
        let d = dossier("recueil");
        let r = Recueil::open_or_init(&d).unwrap();
        assert!(r.morceaux().is_empty());

        let premier = r.nouveau().unwrap();
        let second = r.nouveau().unwrap();
        assert_eq!((premier.as_str(), second.as_str()), ("Sans titre.ly", "Sans titre 2.ly"));
        write_atomic(&r.chemin("Air.ly"), "\\header { title = \"Air de Bach\" }").unwrap();
        // Ni les fichiers cachés, ni les autres extensions.
        write_atomic(&r.chemin(".brouillon.ly"), "").unwrap();
        write_atomic(&r.chemin("notes.txt"), "").unwrap();

        let m = r.morceaux();
        let fichiers: Vec<_> = m.iter().map(|m| m.fichier.as_str()).collect();
        assert_eq!(fichiers, ["Air.ly", "Sans titre 2.ly", "Sans titre.ly"]);
        assert_eq!(m[0].titre, "Air de Bach");
        assert_eq!(nom(&r.nom_libre("a/b")), "a-b.ly");
        fs::remove_dir_all(d).unwrap();
    }
}
