//! La couleur d'accent du bureau : celle que le démon anna de Karenine écrit
//! dans `~/.config/accent/state.json`, suivie à chaque changement. Sans anna,
//! le cuivre d'Ajar.

use std::path::{Path, PathBuf};

use gio::prelude::*;
use gtk::{gio, glib};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accent {
    /// Sur le papier : l'accent assombri, lisible sur un fond clair.
    pub clair: String,
    /// Sur l'encre : l'accent tel quel.
    pub sombre: String,
}

impl Default for Accent {
    /// Le cuivre d'Ajar.
    fn default() -> Self {
        Self {
            clair: "#8a5a2b".into(),
            sombre: "#d4a35f".into(),
        }
    }
}

#[derive(Deserialize)]
struct Etat {
    accent: String,
    accent_dark: Option<String>,
}

pub fn path() -> PathBuf {
    glib::user_config_dir().join("accent").join("state.json")
}

/// Une couleur `#rrggbb`, et rien d'autre : elle finit dans une feuille CSS.
fn couleur(s: &str) -> Option<String> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_lowercase())
}

pub fn lire(texte: &str) -> Option<Accent> {
    let etat: Etat = serde_json::from_str(texte).ok()?;
    let sombre = couleur(&etat.accent)?;
    let clair = etat
        .accent_dark
        .as_deref()
        .and_then(couleur)
        .unwrap_or_else(|| sombre.clone());
    Some(Accent { clair, sombre })
}

/// L'accent du bureau, s'il y en a un.
pub fn bureau(path: &Path) -> Option<Accent> {
    lire(&std::fs::read_to_string(path).ok()?)
}

/// Appelle `changement` à chaque réécriture du fichier. Le moniteur rendu
/// doit rester en vie.
pub fn surveiller(path: &Path, changement: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let moniteur = gio::File::for_path(path)
        .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .ok()?;
    moniteur.connect_changed(move |_, _, _, evenement| {
        use gio::FileMonitorEvent as E;
        // anna écrit puis renomme : on relit une fois le fichier complet.
        if matches!(
            evenement,
            E::ChangesDoneHint | E::Created | E::MovedIn | E::Renamed | E::Deleted
        ) {
            changement();
        }
    });
    Some(moniteur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lecture_de_l_etat_d_anna() {
        let a =
            lire(r##"{"accent":"#7FA6A0","accent_dark":"#4b6360","mode":"dark","palette":{}}"##)
                .unwrap();
        assert_eq!(
            a,
            Accent {
                clair: "#4b6360".into(),
                sombre: "#7fa6a0".into()
            }
        );
        // Sans variante sombre, l'accent sert partout.
        assert_eq!(lire(r##"{"accent":"#c46a5a"}"##).unwrap().clair, "#c46a5a");
        // Ce qui n'est pas une couleur n'entre pas dans la feuille de style.
        assert_eq!(lire(r#"{"accent":"red; } * { color: red"}"#), None);
        assert_eq!(lire("{ en cours d'écriture"), None);
    }
}
