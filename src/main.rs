//! Vinteuil — un pupitre zen pour les partitions LilyPond.
//!
//! Usage : `vinteuil [FICHIER | DOSSIER]`. Un dossier est un recueil ; un
//! `.ly` s'ouvre dans le sien ; un fichier d'un autre format (MusicXML, MIDI,
//! ABC…) est importé dans le dernier recueil, ou dans celui de la fenêtre
//! déjà ouverte. Sans argument, rouvre le dernier morceau, ou crée
//! « Mon recueil » dans ~/Documents/Vinteuil.

mod accent;
mod annotations;
mod app;
mod atelier;
mod config;
mod gravure;
mod import;
mod lilypond;
mod partition;
mod recueil;

use std::path::PathBuf;

use gio::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    let config = config::Config::load();
    let state = config::State::load();

    let dernier = state
        .dernier_recueil
        .clone()
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| {
            glib::user_special_dir(glib::UserDirectory::Documents)
                .unwrap_or_else(glib::home_dir)
                .join("Vinteuil")
                .join("Mon recueil")
        });

    // Le recueil se choisit ici ; le fichier lui-même arrive à la fenêtre par
    // le signal `open`, qu'elle soit déjà ouverte ou non.
    let argument = std::env::args_os().nth(1).map(PathBuf::from);
    let (root, morceau) = match argument {
        None => (dernier, state.dernier_morceau.clone()),
        Some(p) if p.is_dir() => (p, None),
        Some(p) if p.extension().is_some_and(|e| e == "ly") => {
            let dossier = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), PathBuf::from);
            (dossier, Some(recueil::nom(&p)))
        }
        Some(p) if import::Format::detecter(&p).is_some() => {
            (dernier, state.dernier_morceau.clone())
        }
        Some(p) => {
            eprintln!(
                "vinteuil : {} n’est ni un recueil, ni une partition reconnue",
                p.display()
            );
            return glib::ExitCode::FAILURE;
        }
    };

    let recueil = match recueil::Recueil::open_or_init(&root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("vinteuil : impossible d’ouvrir {} : {e:#}", root.display());
            return glib::ExitCode::FAILURE;
        }
    };
    let app = relm4::RelmApp::new("fr.ciph3r.Vinteuil");
    // Un fichier ouvert alors que Vinteuil tourne déjà va à la fenêtre existante.
    relm4::main_application().set_flags(gio::ApplicationFlags::HANDLES_OPEN);
    app.with_args(
        std::env::args_os()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
    )
    .run::<app::App>(app::Init {
        recueil,
        morceau,
        config,
        state,
    });
    glib::ExitCode::SUCCESS
}
