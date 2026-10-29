//! Préférences (`~/.config/vinteuil/config.toml`, éventuellement écrit par
//! Home Manager, donc en lecture seule) et état de la dernière session
//! (`~/.local/state/vinteuil/etat.toml`).

use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::recueil;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Famille de police des titres, des indications et des paroles.
    pub police: String,
    /// Famille de police du source LilyPond.
    pub police_source: String,
    /// Taille du source, en pixels.
    pub taille_source: f64,
    /// Largeur de la ligne de musique, en millimètres.
    pub largeur: f64,
    /// Taille de la portée, en points (20 : celle de LilyPond).
    pub portee: f64,
    /// `papier`, `encre` ou `auto` (suit le thème du système).
    pub theme: String,
    /// Ouvrir l'application en plein écran.
    pub plein_ecran: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            police: "Literata".into(),
            police_source: "monospace".into(),
            taille_source: 15.0,
            largeur: 170.0,
            portee: 20.0,
            theme: "auto".into(),
            plein_ecran: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = glib::user_config_dir().join("vinteuil").join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
                eprintln!(
                    "vinteuil : {} illisible ({e}), préférences par défaut",
                    path.display()
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub dernier_recueil: Option<PathBuf>,
    /// Le morceau ouvert, par son nom de fichier dans le recueil.
    pub dernier_morceau: Option<String>,
    /// Thème choisi dans les réglages ou par `Ctrl+T` (`papier`, `encre` ou
    /// `auto`), prioritaire sur la configuration.
    pub theme: Option<String>,
    /// Agrandissement de la partition (1 : la largeur de la configuration).
    pub zoom: f64,
    /// Prendre la couleur d'accent du bureau (celle de Karenine).
    pub accent: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            dernier_recueil: None,
            dernier_morceau: None,
            theme: None,
            zoom: 1.0,
            accent: true,
        }
    }
}

impl State {
    fn path() -> PathBuf {
        glib::user_state_dir().join("vinteuil").join("etat.toml")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Err(e) = toml::to_string_pretty(self)
            .map_err(anyhow::Error::from)
            .and_then(|t| recueil::write_atomic(&Self::path(), &t))
        {
            eprintln!("vinteuil : état non enregistré ({e})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_partielle() {
        // Ce qui manque prend sa valeur par défaut ; l'inconnu est ignoré.
        let c: Config = toml::from_str("largeur = 150.0\ntheme = \"encre\"\ninconnu = 1").unwrap();
        assert_eq!(c.largeur, 150.0);
        assert_eq!(c.theme, "encre");
        assert_eq!(c.police, "Literata");
        assert_eq!(c.portee, 20.0);
        assert!(c.plein_ecran);
    }

    #[test]
    fn etat_aller_retour() {
        let etat = State {
            dernier_recueil: Some(PathBuf::from("/partitions/Bach")),
            dernier_morceau: Some("Prélude.ly".into()),
            theme: Some("papier".into()),
            zoom: 1.4,
            accent: false,
        };
        let relu: State = toml::from_str(&toml::to_string_pretty(&etat).unwrap()).unwrap();
        assert_eq!(relu.dernier_recueil, etat.dernier_recueil);
        assert_eq!(relu.dernier_morceau.as_deref(), Some("Prélude.ly"));
        assert_eq!(relu.zoom, 1.4);
        assert!(!relu.accent);
        // Un état d'une version antérieure, sans tous les champs.
        let ancien: State = toml::from_str("theme = \"encre\"").unwrap();
        assert_eq!((ancien.zoom, ancien.accent), (1.0, true));
    }
}
