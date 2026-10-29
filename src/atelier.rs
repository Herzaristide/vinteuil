//! L'atelier : un dossier temporaire où travaillent LilyPond et les
//! convertisseurs, et la façon de les lancer sans bloquer la fenêtre.

use std::cell::Cell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{bail, Context, Result};
use gtk::{gio, glib};

pub struct Atelier {
    dossier: PathBuf,
    compteur: Cell<u32>,
}

impl Atelier {
    pub fn new() -> Result<Self> {
        // Un dossier par atelier, même dans un seul processus (les tests).
        static ATELIERS: AtomicU32 = AtomicU32::new(0);
        let n = ATELIERS.fetch_add(1, Ordering::Relaxed);
        let dossier = glib::tmp_dir().join(format!("vinteuil-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dossier).with_context(|| format!("création de {}", dossier.display()))?;
        Ok(Self { dossier, compteur: Cell::new(0) })
    }

    /// Un sous-dossier neuf, pour un seul travail : deux gravures qui se
    /// chevauchent ne mélangent jamais leurs fichiers.
    pub fn etabli(&self) -> Result<Etabli> {
        self.compteur.set(self.compteur.get() + 1);
        let path = self.dossier.join(self.compteur.get().to_string());
        std::fs::create_dir_all(&path)?;
        Ok(Etabli(path))
    }
}

impl Drop for Atelier {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dossier);
    }
}

/// Le sous-dossier d'un travail, vidé quand le travail est fini.
pub struct Etabli(PathBuf);

impl Etabli {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Etabli {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Ce qu'un programme a dit en sortant.
pub struct Sortie {
    pub reussi: bool,
    pub stderr: String,
}

/// Lance `args[0]` dans `cwd`, sans attendre sur la boucle principale.
pub async fn lancer(args: &[&OsStr], cwd: &Path, env: &[(&str, &str)]) -> Result<Sortie> {
    let programme = args.first().context("aucun programme")?.to_string_lossy().into_owned();
    let launcher = gio::SubprocessLauncher::new(
        gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE,
    );
    launcher.set_cwd(cwd);
    for (cle, valeur) in env {
        launcher.setenv(cle, valeur, true);
    }
    if glib::find_program_in_path(&programme).is_none() {
        bail!("« {programme} » est introuvable");
    }
    let process = launcher.spawn(args).with_context(|| format!("lancement de {programme}"))?;
    let (_, stderr) = process.communicate_utf8_future(None).await?;
    Ok(Sortie {
        reussi: process.is_successful(),
        stderr: stderr.map(|s| s.to_string()).unwrap_or_default(),
    })
}

/// Le premier des programmes trouvé dans le `PATH`.
pub fn trouver(noms: &[&str]) -> Option<PathBuf> {
    noms.iter().find_map(glib::find_program_in_path)
}

/// La dernière ligne qui dit quelque chose : pour un échec sans message clair.
pub fn derniere_ligne(texte: &str) -> &str {
    texte.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("échec sans message")
}
