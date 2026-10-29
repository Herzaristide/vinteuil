//! Les annotations à main levée d'un morceau : des traits posés sur la
//! partition, rangés à côté du morceau (`Allemanda.annotations.json`) sans
//! jamais toucher au `.ly`. Les annotations textuelles, elles, vont dans le
//! source, rattachées aux notes (voir `lilypond`).
//!
//! Les positions sont en fractions de la largeur de la page (x de 0 à 1, y de
//! 0 à la hauteur relative de la page) : elles suivent le zoom. Elles ne
//! suivent pas la musique : si la mise en page change, on le signale.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::recueil;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Annotations {
    /// Hauteur relative de chaque page (hauteur / largeur) quand on a annoté :
    /// une autre valeur à l'affichage veut dire que la mise en page a bougé.
    pub proportions: Vec<f64>,
    pub traits: Vec<Trait>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trait {
    pub page: usize,
    /// x, y, et l'épaisseur relative (la pression du stylet).
    pub points: Vec<[f64; 3]>,
}

/// Épaisseur d'un trait, en fraction de la largeur de la page.
pub const EPAISSEUR: f64 = 0.0032;
/// Distance de saisie, en fraction de la largeur de la page.
pub const PRISE: f64 = 0.012;

/// `Allemanda.ly` → `Allemanda.annotations.json`, à côté.
pub fn chemin(morceau: &Path) -> PathBuf {
    morceau.with_extension("annotations.json")
}

impl Annotations {
    pub fn lire(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => serde_json::from_str(&t).with_context(|| format!("lecture de {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("lecture de {}", path.display())),
        }
    }

    /// Écrit les annotations ; sans annotation, le fichier disparaît.
    pub fn ecrire(&self, path: &Path) -> Result<()> {
        if self.traits.is_empty() {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            };
        }
        recueil::write_atomic(path, &serde_json::to_string_pretty(self)?)
    }

    /// La mise en page a-t-elle bougé depuis qu'on a annoté ?
    pub fn decalees(&self, proportions: &[f64]) -> bool {
        !self.traits.is_empty()
            && !self.proportions.is_empty()
            && (self.proportions.len() != proportions.len()
                || self.proportions.iter().zip(proportions).any(|(a, b)| (a - b).abs() > 0.002))
    }

    /// Le trait le plus proche de (x, y) sur `page`, s'il est à portée.
    pub fn trait_proche(&self, page: usize, x: f64, y: f64) -> Option<usize> {
        let distance = |t: &Trait| {
            t.points
                .iter()
                .map(|p| (p[0] - x).hypot(p[1] - y))
                .fold(f64::INFINITY, f64::min)
        };
        self.traits
            .iter()
            .enumerate()
            .filter(|(_, t)| t.page == page)
            .map(|(i, t)| (i, distance(t)))
            .filter(|(_, d)| *d <= PRISE)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trait_(page: usize, points: &[(f64, f64)]) -> Trait {
        Trait { page, points: points.iter().map(|&(x, y)| [x, y, 1.0]).collect() }
    }

    #[test]
    fn chemin_a_cote_du_morceau() {
        assert_eq!(chemin(Path::new("/r/Air.ly")), PathBuf::from("/r/Air.annotations.json"));
    }

    #[test]
    fn le_trait_le_plus_proche() {
        let a = Annotations {
            traits: vec![trait_(0, &[(0.1, 0.1), (0.2, 0.1)]), trait_(0, &[(0.5, 0.5)]), trait_(1, &[(0.1, 0.1)])],
            ..Default::default()
        };
        // Trop loin de tout.
        assert_eq!(a.trait_proche(0, 0.9, 0.9), None);
        assert_eq!(a.trait_proche(0, 0.205, 0.105), Some(0));
        assert_eq!(a.trait_proche(0, 0.505, 0.5), Some(1));
        // Celui de la page 1 n'est pas sur la page 0.
        assert_eq!(a.trait_proche(1, 0.1, 0.1), Some(2));
    }

    #[test]
    fn decalage_de_la_mise_en_page() {
        let mut a = Annotations { proportions: vec![1.5], ..Default::default() };
        // Sans annotation, rien à signaler.
        assert!(!a.decalees(&[2.0]));
        a.traits.push(trait_(0, &[(0.1, 0.1)]));
        assert!(!a.decalees(&[1.5005]));
        assert!(a.decalees(&[1.6]));
        assert!(a.decalees(&[1.5, 0.4]));
    }

    #[test]
    fn aller_retour_sur_disque() {
        let dir = std::env::temp_dir().join(format!("vinteuil-annotations-{}", std::process::id()));
        let path = chemin(&dir.join("Air.ly"));
        assert_eq!(Annotations::lire(&path).unwrap(), Annotations::default());

        let a = Annotations { proportions: vec![1.2], traits: vec![trait_(0, &[(0.1, 0.2)])] };
        a.ecrire(&path).unwrap();
        assert_eq!(Annotations::lire(&path).unwrap(), a);
        // L'ancien format, avec des textes libres, se lit encore.
        std::fs::write(&path, r#"{"traits":[],"textes":[{"page":0,"x":0,"y":0,"texte":"4"}]}"#).unwrap();
        assert!(Annotations::lire(&path).unwrap().traits.is_empty());
        // Tout effacé : le fichier disparaît.
        Annotations::default().ecrire(&path).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
