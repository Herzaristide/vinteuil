//! La fenêtre : la partition au centre, les morceaux du recueil à gauche
//! (`Alt+A`), le source LilyPond à droite (`Alt+E`). Les panneaux flottent sur
//! les marges et ne déplacent jamais la partition. Le source s'enregistre et
//! se regrave après une courte pause dans la frappe. `Ctrl+O` importe un
//! fichier d'un autre format ; on peut aussi le déposer sur la fenêtre.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use relm4::prelude::*;

use crate::accent::{self, Accent};
use crate::atelier::Atelier;
use crate::config::{Config, State};
use crate::gravure::Lien;
use crate::gravure::{self, Erreur};
use crate::import::{self, Format};
use crate::lilypond::{self, Cible, Edition};
use crate::partition::{self as surface, Evenement, Partition, Pointe};
use crate::recueil::{self, Morceau, Recueil};

/// Délai entre la dernière frappe et l'enregistrement, suivi de la gravure.
const SAVE_DELAY_MS: u64 = 700;
const ZOOM_MIN: f64 = 0.5;
const ZOOM_MAX: f64 = 3.0;

const ENCRE_PAPIER: &str = "#2a2521";
const ENCRE_ENCRE: &str = "#d9d2c5";
const RECUEIL_VIDE: &str = "Ce recueil est vide.\n\nCtrl+N pour écrire un morceau,\nCtrl+O ou un glisser-déposer pour en importer un.";

pub struct Init {
    pub recueil: Recueil,
    /// Le morceau à ouvrir, par son nom de fichier.
    pub morceau: Option<String>,
    pub config: Config,
    pub state: State,
}

#[derive(Debug)]
pub enum Msg {
    ToggleMorceaux,
    ToggleSource,
    CloseAll,
    /// Ouvre les réglages.
    Options,
    /// Ouvre un morceau du recueil, par son nom de fichier.
    Ouvrir(String),
    Nouveau,
    /// Demande le fichier à importer.
    DemanderImport,
    /// Un fichier passé à l'application (ligne de commande, « Ouvrir avec »).
    Recevoir(PathBuf),
    Importer(PathBuf),
    Importe(Result<String, String>),
    Edited,
    Save(u64),
    Graver,
    Grave(u64, Result<Vec<gravure::Page>, Erreur>),
    /// Multiplie l'agrandissement ; `None` revient à 1.
    Zoom(Option<f64>),
    ToggleTheme,
    /// Sombre (encre), clair (papier), ou (`None`) comme le système.
    Theme(Option<bool>),
    /// Le système est passé du clair au sombre, ou l'inverse.
    Systeme,
    /// Prendre, ou non, l'accent du bureau.
    SuivreAccent(bool),
    /// L'accent du bureau a peut-être changé.
    Accent,
    /// Ce que demande la partition : un menu, un message, l'annulation…
    Partition(Evenement),
    /// Rattache un texte à la note qui commence là, au-dessus ou en dessous.
    AnnoterNote {
        ligne: usize,
        colonne: usize,
        dessous: bool,
        texte: String,
    },
    RetirerAnnotation {
        ligne: usize,
        colonne: usize,
    },
    EffacerTrait(usize),
    MontrerAnnotations,
    /// Le défilement automatique, un cran vers le bas (`1`) ou vers le haut (`-1`).
    Defiler(i32),
    ToggleFullscreen,
    Quit,
}

/// Ce qu'une annulation défait : un trait, ou un texte dans le source.
#[derive(Debug, Clone, Copy)]
enum Genre {
    Trait,
    Texte,
}

pub struct App {
    recueil: Recueil,
    config: Config,
    state: State,
    atelier: Rc<Atelier>,
    morceaux: Vec<Morceau>,
    /// Le morceau ouvert, par son nom de fichier.
    courant: Option<String>,
    dirty: bool,
    save_generation: u64,
    gravure_generation: u64,
    /// Les liens de la partition affichée mènent au source tel qu'il est.
    liens_a_jour: bool,
    /// L'annulation commune aux traits et aux textes : ce qu'on peut annuler,
    /// puis refaire, du plus ancien au plus récent.
    annulables: Vec<Genre>,
    refaisables: Vec<Genre>,
    /// Le message de vitesse du défilement : remplacé, pas empilé, à chaque appui.
    message_defilement: Option<adw::Toast>,
    /// Vrai pendant qu'on charge un source : ce n'est pas une modification.
    loading: Rc<Cell<bool>>,
    dark: bool,
    /// Le thème suit celui du système.
    auto: bool,
    accent_css: gtk::CssProvider,
    _moniteur: Option<gio::FileMonitor>,
}

pub struct Widgets {
    toasts: adw::ToastOverlay,
    partition: Partition,
    source: gtk::TextView,
    source_titre: gtk::Label,
    erreur: gtk::Label,
    left: gtk::Revealer,
    right: gtk::Revealer,
    reglages: gtk::Revealer,
    theme: adw::ToggleGroup,
    liste: gtk::ListBox,
}

impl Component for App {
    type CommandOutput = ();
    type Input = Msg;
    type Output = ();
    type Init = Init;
    type Root = adw::ApplicationWindow;
    type Widgets = Widgets;

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .title("Vinteuil")
            .default_width(1280)
            .default_height(860)
            .css_classes(["vinteuil"])
            .build()
    }

    fn init(init: Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let Init {
            recueil,
            morceau,
            config,
            state,
        } = init;
        load_css(&config);
        let accent_css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("aucun écran"),
            &accent_css,
            gtk::STYLE_PROVIDER_PRIORITY_USER + 2,
        );

        let atelier = match Atelier::new() {
            Ok(a) => Rc::new(a),
            Err(e) => panic!("vinteuil : atelier temporaire impossible : {e:#}"),
        };
        let loading = Rc::new(Cell::new(false));

        // La partition.
        let partition = Partition::new(
            gravure::largeur_page(config.largeur),
            glib::clone!(
                #[strong]
                sender,
                move |evenement| sender.input(Msg::Partition(evenement))
            ),
        );
        partition.zoomer(state.zoom);
        let zoom_souris =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        zoom_souris.connect_scroll(glib::clone!(
            #[strong]
            sender,
            move |c, _, dy| {
                if !c
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK)
                {
                    return glib::Propagation::Proceed;
                }
                sender.input(Msg::Zoom(Some(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 })));
                glib::Propagation::Stop
            }
        ));
        zoom_souris.set_propagation_phase(gtk::PropagationPhase::Capture);
        partition.scrolled.add_controller(zoom_souris);

        // Les morceaux, à gauche.
        let liste = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["morceaux"])
            .build();
        liste.connect_row_activated(glib::clone!(
            #[strong]
            sender,
            move |_, row| sender.input(Msg::Ouvrir(row.widget_name().to_string()))
        ));
        let left_box = panel_box("Morceaux", &recueil.titre());
        left_box.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&liste)
                .build(),
        );
        left_box.append(
            &gtk::Label::builder()
                .label("Ctrl+N nouveau · Ctrl+O importer")
                .xalign(0.0)
                .css_classes(["dim-label", "caption"])
                .build(),
        );
        let left = side_revealer(
            &left_box,
            gtk::Align::Start,
            gtk::RevealerTransitionType::SlideRight,
        );

        // Le source, à droite.
        let source = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .monospace(true)
            .css_classes(["source"])
            .build();
        source.buffer().set_enable_undo(true);
        source.buffer().create_tag(
            Some("erreur"),
            &[("underline", &gtk::pango::Underline::Error)],
        );
        let source_titre = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["dim-label", "caption"])
            .build();
        let erreur = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .selectable(true)
            .visible(false)
            .css_classes(["erreur", "caption"])
            .build();
        let right_box = panel_box("Source", "");
        right_box.set_width_request(520);
        right_box.append(&source_titre);
        right_box.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&source)
                .build(),
        );
        right_box.append(&erreur);
        let right = side_revealer(
            &right_box,
            gtk::Align::End,
            gtk::RevealerTransitionType::SlideLeft,
        );

        // Les réglages, au centre, du même papier que les panneaux.
        // Les noms des positions sont ceux de la configuration.
        let theme = adw::ToggleGroup::builder()
            .homogeneous(true)
            .css_classes(["choix"])
            .build();
        for (nom, label) in [
            ("auto", "Système"),
            ("papier", "Clair"),
            ("encre", "Sombre"),
        ] {
            theme.add(adw::Toggle::builder().name(nom).label(label).build());
        }
        let suivre_accent = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let reglages_box = panel_box(
            "Réglages",
            "S’appliquent aussitôt, et restent d’une séance à l’autre.",
        );
        reglages_box.set_width_request(380);
        reglages_box.append(&reglage_dessous(
            "Thème",
            "Comme le bureau, le papier ou l’encre · Ctrl+T",
            &theme,
        ));
        reglages_box.append(&reglage(
            "Accent du bureau",
            "La couleur choisie dans Karenine, plutôt que le cuivre",
            &suivre_accent,
        ));
        let reglages = gtk::Revealer::builder()
            .can_target(false)
            .child(&reglages_box)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(220)
            .build();

        source.buffer().connect_changed(glib::clone!(
            #[strong]
            sender,
            #[strong]
            loading,
            move |_| {
                if !loading.get() {
                    sender.input(Msg::Edited);
                }
            }
        ));

        let overlay = gtk::Overlay::builder().child(&partition.scrolled).build();
        overlay.add_overlay(&left);
        overlay.add_overlay(&right);
        overlay.add_overlay(&reglages);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&overlay));
        toasts.add_css_class("fond");
        root.set_content(Some(&toasts));

        // Un fichier déposé sur la fenêtre est importé.
        let depot = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        depot.connect_drop(glib::clone!(
            #[strong]
            sender,
            move |_, valeur, _, _| {
                let Ok(fichiers) = valeur.get::<gdk::FileList>() else {
                    return false;
                };
                let chemins: Vec<_> = fichiers.files().iter().filter_map(|f| f.path()).collect();
                for chemin in &chemins {
                    sender.input(Msg::Importer(chemin.clone()));
                }
                !chemins.is_empty()
            }
        ));
        toasts.add_controller(depot);

        // Les fichiers passés à l'application, au lancement ou plus tard.
        relm4::main_application().connect_open(glib::clone!(
            #[strong]
            sender,
            move |_, fichiers, _| {
                for chemin in fichiers.iter().filter_map(|f| f.path()) {
                    sender.input(Msg::Recevoir(chemin));
                }
            }
        ));

        install_shortcuts(&root, &sender);
        root.connect_close_request(glib::clone!(
            #[strong]
            sender,
            move |_| {
                sender.input(Msg::Quit);
                glib::Propagation::Stop
            }
        ));

        let voulu = state.theme.clone().unwrap_or_else(|| config.theme.clone());
        let choix = match voulu.as_str() {
            "encre" => Some(true),
            "papier" => Some(false),
            _ => None,
        };
        let moniteur = accent::surveiller(
            &accent::path(),
            glib::clone!(
                #[strong]
                sender,
                move || sender.input(Msg::Accent)
            ),
        );

        let mut model = App {
            recueil,
            config,
            state,
            atelier,
            morceaux: Vec::new(),
            courant: None,
            dirty: false,
            save_generation: 0,
            gravure_generation: 0,
            liens_a_jour: false,
            annulables: Vec::new(),
            refaisables: Vec::new(),
            message_defilement: None,
            loading,
            dark: apply_theme(&root, choix),
            auto: choix.is_none(),
            accent_css,
            _moniteur: moniteur,
        };
        model.appliquer_accent(&partition);

        // Les interrupteurs partent de l'état actuel, puis seulement on les écoute.
        theme.set_active_name(Some(nom_theme(model.auto, model.dark)));
        suivre_accent.set_active(model.state.accent);
        theme.connect_active_name_notify(glib::clone!(
            #[strong]
            sender,
            move |g| {
                let choix = match g.active_name().as_deref() {
                    Some("encre") => Some(true),
                    Some("papier") => Some(false),
                    _ => None,
                };
                sender.input(Msg::Theme(choix))
            }
        ));
        suivre_accent.connect_active_notify(glib::clone!(
            #[strong]
            sender,
            move |s| sender.input(Msg::SuivreAccent(s.is_active()))
        ));
        adw::StyleManager::default().connect_dark_notify(glib::clone!(
            #[strong]
            sender,
            move |_| sender.input(Msg::Systeme)
        ));

        if model.config.plein_ecran {
            root.fullscreen();
        }
        root.present();

        let widgets = Widgets {
            toasts,
            partition,
            source,
            source_titre,
            erreur,
            left,
            right,
            reglages,
            theme,
            liste,
        };

        model.relire(&widgets);
        let premier = morceau
            .filter(|m| model.morceaux.iter().any(|x| &x.fichier == m))
            .or_else(|| model.morceaux.first().map(|m| m.fichier.clone()));
        match premier {
            Some(m) => sender.input(Msg::Ouvrir(m)),
            None => widgets.partition.vide(RECUEIL_VIDE),
        }
        widgets.partition.grab_focus();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        w: &mut Self::Widgets,
        msg: Msg,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match msg {
            Msg::ToggleMorceaux => {
                let open = !w.left.reveals_child();
                reveler(&w.left, open);
                if open {
                    w.liste.grab_focus();
                } else {
                    w.partition.grab_focus();
                }
            }
            Msg::ToggleSource => {
                let open = !w.right.reveals_child();
                reveler(&w.right, open);
                if open {
                    w.source.grab_focus();
                } else {
                    w.partition.grab_focus();
                }
            }
            Msg::CloseAll => {
                w.partition.arreter_defilement();
                reveler(&w.reglages, false);
                reveler(&w.left, false);
                reveler(&w.right, false);
                w.partition.grab_focus();
            }
            Msg::Options => {
                let open = !w.reglages.reveals_child();
                reveler(&w.reglages, open);
                if open {
                    w.theme.grab_focus();
                } else {
                    w.partition.grab_focus();
                }
            }

            Msg::Ouvrir(fichier) => {
                // Déjà ouvert (au lancement, le fichier arrive aussi par `open`).
                if self.courant.as_deref() == Some(fichier.as_str()) {
                    reveler(&w.left, false);
                    return;
                }
                // Le morceau qu'on quitte part d'abord sur le disque.
                if !self.save(w) {
                    return;
                }
                let texte = match recueil::read(&self.recueil.chemin(&fichier)) {
                    Ok(t) => t,
                    Err(e) => return toast(w, &format!("{e:#}")),
                };
                self.with_loading(|| w.source.buffer().set_text(&texte));
                w.source
                    .buffer()
                    .place_cursor(&w.source.buffer().start_iter());
                w.source_titre.set_label(&fichier);
                w.erreur.set_visible(false);
                w.partition
                    .charger_annotations(&self.recueil.chemin(&fichier));
                self.liens_a_jour = false;
                self.annulables.clear();
                self.refaisables.clear();
                self.courant = Some(fichier.clone());
                self.state.dernier_morceau = Some(fichier);
                self.reconstruire_liste(w);
                // Pas l'ancien morceau sous le nouveau titre : une gravure peut être longue.
                w.partition.vide("Gravure…");
                // Depuis le panneau : on retourne à la partition.
                if w.left.reveals_child() {
                    reveler(&w.left, false);
                    w.partition.grab_focus();
                }
                sender.input(Msg::Graver);
            }
            Msg::Nouveau => match self.recueil.nouveau() {
                Ok(fichier) => {
                    self.relire(w);
                    sender.input(Msg::Ouvrir(fichier));
                    reveler(&w.right, true);
                    w.source.grab_focus();
                }
                Err(e) => toast(w, &format!("Échec de la création : {e:#}")),
            },
            Msg::DemanderImport => demander_import(root, &sender),
            Msg::Recevoir(chemin) => {
                root.present();
                let chemin = chemin.canonicalize().unwrap_or(chemin);
                if chemin.is_dir() {
                    if chemin != self.recueil.root {
                        toast(
                            w,
                            &format!("Un recueil est déjà ouvert : {}", self.recueil.titre()),
                        );
                    }
                } else if chemin.parent() == Some(self.recueil.root.as_path())
                    && Format::detecter(&chemin) == Some(Format::LilyPond)
                {
                    sender.input(Msg::Ouvrir(recueil::nom(&chemin)));
                } else {
                    sender.input(Msg::Importer(chemin));
                }
            }
            Msg::Importer(chemin) => {
                let Some(format) = Format::detecter(&chemin) else {
                    return toast(w, &format!("Format inconnu : {}", recueil::nom(&chemin)));
                };
                if format != Format::LilyPond {
                    toast(w, &format!("Conversion depuis {}…", format.nom()));
                }
                let (atelier, root_recueil, sender) = (
                    self.atelier.clone(),
                    self.recueil.root.clone(),
                    sender.clone(),
                );
                glib::spawn_future_local(async move {
                    let recueil = Recueil { root: root_recueil };
                    let resultat = import::importer(&atelier, &recueil, &chemin)
                        .await
                        .map_err(|e| format!("{e:#}"));
                    sender.input(Msg::Importe(resultat));
                });
            }
            Msg::Importe(resultat) => match resultat {
                Ok(fichier) => {
                    self.relire(w);
                    toast(
                        w,
                        &format!("« {} » ajouté au recueil", fichier.trim_end_matches(".ly")),
                    );
                    sender.input(Msg::Ouvrir(fichier));
                }
                Err(e) => toast(w, &format!("Import impossible : {e}")),
            },

            Msg::Edited => {
                self.dirty = true;
                self.liens_a_jour = false;
                self.save_generation += 1;
                let generation = self.save_generation;
                let sender = sender.clone();
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(SAVE_DELAY_MS),
                    move || sender.input(Msg::Save(generation)),
                );
            }
            Msg::Save(generation) => {
                if generation == self.save_generation && self.dirty && self.save(w) {
                    self.relire(w);
                    sender.input(Msg::Graver);
                }
            }
            Msg::Graver => {
                let Some(fichier) = &self.courant else { return };
                self.gravure_generation += 1;
                let generation = self.gravure_generation;
                w.partition.en_gravure(true);
                let reglages = gravure::Reglages {
                    largeur: self.config.largeur,
                    portee: self.config.portee,
                    police: self.config.police.clone(),
                };
                let (atelier, chemin, sender) = (
                    self.atelier.clone(),
                    self.recueil.chemin(fichier),
                    sender.clone(),
                );
                glib::spawn_future_local(async move {
                    let resultat = gravure::graver(&atelier, &chemin, &reglages).await;
                    sender.input(Msg::Grave(generation, resultat));
                });
            }
            Msg::Grave(generation, resultat) => {
                // Une gravure plus récente est en route : celle-ci est déjà dépassée.
                if generation != self.gravure_generation {
                    return;
                }
                w.partition.en_gravure(false);
                let buffer = w.source.buffer();
                buffer.remove_tag_by_name("erreur", &buffer.start_iter(), &buffer.end_iter());
                match resultat {
                    Ok(pages) => {
                        w.erreur.set_visible(false);
                        self.liens_a_jour = !self.dirty;
                        if let Err(e) = w.partition.afficher(pages) {
                            toast(w, &format!("Partition illisible : {e}"));
                        }
                    }
                    // La dernière partition réussie reste affichée.
                    Err(e) => {
                        if let Some(ligne) = e.ligne.and_then(|l| buffer.iter_at_line(l as i32 - 1))
                        {
                            let mut fin = ligne;
                            fin.forward_to_line_end();
                            buffer.apply_tag_by_name("erreur", &ligne, &fin);
                        }
                        w.erreur.set_label(&e.to_string());
                        w.erreur.set_visible(true);
                        if w.partition.est_vide() {
                            w.partition.vide(&format!("La partition n’a pas pu être gravée.\n\n{e}\n\nAlt+E pour voir le source."));
                        }
                        if !w.right.reveals_child() {
                            toast(w, &format!("Gravure impossible · {e}"));
                        }
                    }
                }
            }

            Msg::Zoom(facteur) => {
                let zoom = facteur
                    .map_or(1.0, |f| self.state.zoom * f)
                    .clamp(ZOOM_MIN, ZOOM_MAX);
                if zoom != self.state.zoom {
                    self.state.zoom = zoom;
                    w.partition.zoomer(zoom);
                    self.state.save();
                }
            }
            // Choisir à la main, c'est cesser de suivre le système.
            Msg::ToggleTheme => sender.input(Msg::Theme(Some(!self.dark))),
            Msg::Theme(choix) => {
                let nom = nom_theme(choix.is_none(), choix.unwrap_or(self.dark));
                // Ctrl+T aussi : le sélecteur suit (sans boucle, la valeur est déjà la bonne).
                w.theme.set_active_name(Some(nom));
                if choix.is_none() != self.auto || choix.is_some_and(|d| d != self.dark) {
                    self.auto = choix.is_none();
                    self.changer_theme(w, root, choix);
                    self.state.theme = Some(nom.into());
                    self.state.save();
                }
            }
            Msg::Systeme => {
                if self.auto {
                    self.changer_theme(w, root, None);
                }
            }
            Msg::SuivreAccent(on) => {
                self.state.accent = on;
                self.state.save();
                self.appliquer_accent(&w.partition);
            }
            Msg::Accent => self.appliquer_accent(&w.partition),
            Msg::Partition(evenement) => match evenement {
                Evenement::Signaler(message) => toast(w, &message),
                Evenement::Annoter(pointe) => self.annoter(w, pointe, &sender),
                Evenement::Gommer(pointe) => self.gommer(w, pointe, &sender),
                Evenement::Trace => self.retenir(Genre::Trait),
                Evenement::Annuler => self.remonter(w, &sender, true),
                Evenement::Refaire => self.remonter(w, &sender, false),
            },
            Msg::AnnoterNote {
                ligne,
                colonne,
                dessous,
                texte,
            } => {
                let source = texte_de(&w.source);
                match lilypond::annoter(&source, ligne, colonne, dessous, &texte) {
                    Some(editions) => self.editer(w, &sender, editions),
                    None => toast(w, "Cette note ne peut pas être annotée"),
                }
            }
            Msg::RetirerAnnotation { ligne, colonne } => {
                if let Some(editions) = lilypond::retirer(&texte_de(&w.source), ligne, colonne) {
                    self.editer(w, &sender, editions);
                }
            }
            Msg::EffacerTrait(rang) => w.partition.effacer_trait(rang),
            Msg::Defiler(sens) => {
                let cran = w.partition.defiler(sens);
                let texte = match cran {
                    0 => "Défilement arrêté".to_string(),
                    c if c > 0 => {
                        format!("Défilement ↓ {c} · Alt+↑ pour ralentir, Échap pour arrêter")
                    }
                    c => format!(
                        "Défilement ↑ {} · Alt+↓ pour ralentir, Échap pour arrêter",
                        -c
                    ),
                };
                if let Some(ancien) = self.message_defilement.take() {
                    ancien.dismiss();
                }
                let message = adw::Toast::builder().title(texte).timeout(2).build();
                w.toasts.add_toast(message.clone());
                self.message_defilement = Some(message);
            }
            Msg::MontrerAnnotations => {
                let visibles = w.partition.basculer_annotations();
                toast(
                    w,
                    if visibles {
                        "Annotations visibles"
                    } else {
                        "Annotations masquées · Alt+H"
                    },
                );
            }
            Msg::ToggleFullscreen => root.set_fullscreened(!root.is_fullscreen()),

            Msg::Quit => {
                if self.save(w) {
                    self.state.dernier_recueil = Some(self.recueil.root.clone());
                    self.state.save();
                    relm4::main_application().quit();
                } else {
                    toast(
                        w,
                        "Fermeture annulée : le morceau n’a pas pu être enregistré",
                    );
                }
            }
        }
    }
}

impl App {
    /// Ce que désignent les liens pointés, dans le source tel qu'il est. Rien
    /// si le source a changé depuis la gravure : les liens mèneraient ailleurs.
    fn cibles(&self, w: &Widgets, p: &Pointe) -> Vec<(Lien, Cible)> {
        if !self.liens_a_jour {
            if !p.liens.is_empty() {
                toast(w, "Un instant : la partition se regrave");
            }
            return Vec::new();
        }
        let source = texte_de(&w.source);
        p.liens
            .iter()
            .map(|l| (l.clone(), lilypond::cible(&source, l.ligne, l.colonne)))
            .collect()
    }

    /// Un clic sur une note : un texte à y rattacher, au-dessus (en dessous avec `Maj`).
    fn annoter(&self, w: &Widgets, p: Pointe, sender: &ComponentSender<Self>) {
        let Some((note, _)) = self
            .cibles(w, &p)
            .into_iter()
            .find(|(_, c)| *c == Cible::Note)
        else {
            return;
        };
        let (ligne, colonne, dessous, sender) = (note.ligne, note.colonne, p.maj, sender.clone());
        let indication = if dessous {
            "En dessous : doigté, nuance…"
        } else {
            "Doigté, nuance, remarque…"
        };
        surface::bulle_texte(&p.zone, p.x, p.y, indication, move |texte| {
            sender.input(Msg::AnnoterNote {
                ligne,
                colonne,
                dessous,
                texte,
            })
        });
    }

    /// Un clic droit : retire l'annotation pointée, sinon efface le trait pointé.
    fn gommer(&self, w: &Widgets, p: Pointe, sender: &ComponentSender<Self>) {
        let annotation = self
            .cibles(w, &p)
            .into_iter()
            .find(|(_, c)| matches!(c, Cible::Annotation(_)));
        if let Some((lien, _)) = annotation {
            sender.input(Msg::RetirerAnnotation {
                ligne: lien.ligne,
                colonne: lien.colonne,
            });
        } else if let Some(rang) = p.trait_ {
            sender.input(Msg::EffacerTrait(rang));
        }
    }

    /// Une modification qu'on pourra annuler ; elle efface ce qu'on pouvait refaire.
    fn retenir(&mut self, genre: Genre) {
        self.annulables.push(genre);
        self.refaisables.clear();
    }

    /// `annuler` : défait la dernière annotation, trait ou texte ; sinon la refait.
    fn remonter(&mut self, w: &Widgets, sender: &ComponentSender<Self>, annuler: bool) {
        let genre = if annuler {
            self.annulables.pop()
        } else {
            self.refaisables.pop()
        };
        let Some(genre) = genre else { return };
        if annuler {
            self.refaisables.push(genre);
        } else {
            self.annulables.push(genre);
        }
        match genre {
            Genre::Trait => w.partition.remonter(annuler),
            Genre::Texte => {
                let buffer = w.source.buffer();
                self.with_loading(|| {
                    if annuler {
                        buffer.undo()
                    } else {
                        buffer.redo()
                    }
                });
                self.enregistrer_et_graver(w, sender);
            }
        }
    }

    /// Applique au source les éditions d'une annotation, d'un seul tenant
    /// (une seule annulation), puis enregistre et regrave aussitôt.
    fn editer(&mut self, w: &Widgets, sender: &ComponentSender<Self>, editions: Vec<Edition>) {
        let buffer = w.source.buffer();
        self.with_loading(|| {
            buffer.begin_user_action();
            for e in editions {
                match e {
                    Edition::Inserer {
                        ligne,
                        colonne,
                        texte,
                    } => {
                        let mut it = buffer
                            .iter_at_line_offset(ligne as i32, colonne as i32)
                            .unwrap_or(buffer.end_iter());
                        buffer.insert(&mut it, &texte);
                    }
                    Edition::Supprimer {
                        ligne,
                        colonne,
                        longueur,
                    } => {
                        let Some(mut debut) =
                            buffer.iter_at_line_offset(ligne as i32, colonne as i32)
                        else {
                            continue;
                        };
                        let mut fin = debut;
                        fin.forward_chars(longueur as i32);
                        buffer.delete(&mut debut, &mut fin);
                    }
                }
            }
            buffer.end_user_action();
        });
        self.retenir(Genre::Texte);
        self.enregistrer_et_graver(w, sender);
    }

    /// Le source vient de changer sans passer par la frappe : enregistré et
    /// regravé sans attendre la pause habituelle.
    fn enregistrer_et_graver(&mut self, w: &Widgets, sender: &ComponentSender<Self>) {
        self.dirty = true;
        self.liens_a_jour = false;
        // Une frappe en attente d'enregistrement n'a plus à l'être.
        self.save_generation += 1;
        if self.save(w) {
            sender.input(Msg::Graver);
        }
    }

    fn with_loading(&self, f: impl FnOnce()) {
        self.loading.set(true);
        f();
        self.loading.set(false);
    }

    /// Enregistre le source s'il a changé. Renvoie `false` en cas d'échec.
    fn save(&mut self, w: &Widgets) -> bool {
        let (true, Some(fichier)) = (self.dirty, &self.courant) else {
            return true;
        };
        let buffer = w.source.buffer();
        let texte = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        match recueil::write_atomic(&self.recueil.chemin(fichier), &texte) {
            Ok(()) => {
                self.dirty = false;
                true
            }
            Err(e) => {
                toast(w, &format!("Échec de l’enregistrement : {e:#}"));
                false
            }
        }
    }

    /// Relit la liste des morceaux (un titre a pu changer, un fichier arriver).
    fn relire(&mut self, w: &Widgets) {
        let morceaux = self.recueil.morceaux();
        if morceaux != self.morceaux || w.liste.first_child().is_none() {
            self.morceaux = morceaux;
            self.reconstruire_liste(w);
        }
    }

    fn reconstruire_liste(&self, w: &Widgets) {
        while let Some(row) = w.liste.first_child() {
            w.liste.remove(&row);
        }
        for m in &self.morceaux {
            w.liste.append(&morceau_row(
                m,
                self.courant.as_deref() == Some(m.fichier.as_str()),
            ));
        }
    }

    fn encre(&self) -> &'static str {
        if self.dark {
            ENCRE_ENCRE
        } else {
            ENCRE_PAPIER
        }
    }

    fn changer_theme(&mut self, w: &Widgets, root: &adw::ApplicationWindow, choix: Option<bool>) {
        self.dark = apply_theme(root, choix);
        self.appliquer_accent(&w.partition);
    }

    /// L'accent du bureau si on le suit et qu'il existe, le cuivre sinon :
    /// pour l'interface, et pour les annotations (vif sur l'encre, assombri
    /// sur le papier).
    fn appliquer_accent(&self, partition: &Partition) {
        let accent = self
            .state
            .accent
            .then(|| accent::bureau(&accent::path()))
            .flatten()
            .unwrap_or_default();
        self.accent_css.load_from_string(&css_accent(&accent));
        partition.couleurs(
            self.encre(),
            if self.dark {
                &accent.sombre
            } else {
                &accent.clair
            },
        );
    }
}

/// Demande le fichier à importer, filtré sur les formats reconnus.
fn demander_import(root: &adw::ApplicationWindow, sender: &ComponentSender<App>) {
    let filtres = gio::ListStore::new::<gtk::FileFilter>();
    let tous = gtk::FileFilter::new();
    tous.set_name(Some("Toutes les partitions"));
    for (_, extensions) in Format::EXTENSIONS {
        for ext in extensions {
            tous.add_suffix(ext);
        }
    }
    filtres.append(&tous);
    for (format, extensions) in Format::EXTENSIONS {
        let filtre = gtk::FileFilter::new();
        filtre.set_name(Some(&format!(
            "{} ({})",
            format.nom(),
            extensions.join(", ")
        )));
        for ext in extensions {
            filtre.add_suffix(ext);
        }
        filtres.append(&filtre);
    }
    let dialog = gtk::FileDialog::builder()
        .title("Importer une partition")
        .accept_label("Importer")
        .filters(&filtres)
        .default_filter(&tous)
        .build();
    let sender = sender.clone();
    dialog.open(Some(root), gio::Cancellable::NONE, move |r| {
        if let Some(chemin) = r.ok().and_then(|f| f.path()) {
            sender.input(Msg::Importer(chemin));
        }
    });
}

fn toast(w: &Widgets, text: &str) {
    w.toasts.add_toast(adw::Toast::new(text));
}

/// Sombre, clair, ou (`None`) comme le système ; rend ce qui en résulte.
fn apply_theme(root: &adw::ApplicationWindow, choix: Option<bool>) -> bool {
    let style = adw::StyleManager::default();
    style.set_color_scheme(match choix {
        Some(true) => adw::ColorScheme::ForceDark,
        Some(false) => adw::ColorScheme::ForceLight,
        None => adw::ColorScheme::Default,
    });
    let dark = style.is_dark();
    root.remove_css_class(if dark { "papier" } else { "encre" });
    root.add_css_class(if dark { "encre" } else { "papier" });
    dark
}

/// Le nom du thème, dans la configuration comme dans le sélecteur.
fn nom_theme(auto: bool, dark: bool) -> &'static str {
    match (auto, dark) {
        (true, _) => "auto",
        (false, true) => "encre",
        (false, false) => "papier",
    }
}

fn morceau_row(m: &Morceau, current: bool) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(&m.titre)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let row = gtk::ListBoxRow::builder()
        .child(&label)
        .name(&m.fichier)
        .tooltip_text(&m.fichier)
        .build();
    if current {
        row.add_css_class("courant");
    }
    row
}

/// Une ligne de réglage : intitulé et explication à gauche, interrupteur à droite.
fn reglage(titre: &str, detail: &str, interrupteur: &impl IsA<gtk::Widget>) -> gtk::Box {
    let ligne = gtk::Box::builder()
        .spacing(18)
        .css_classes(["reglage"])
        .build();
    ligne.append(&intitule(titre, detail));
    ligne.append(interrupteur);
    ligne
}

/// Un réglage dont le choix, trop large pour la marge, passe dessous.
fn reglage_dessous(titre: &str, detail: &str, choix: &impl IsA<gtk::Widget>) -> gtk::Box {
    let bloc = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .css_classes(["reglage"])
        .build();
    bloc.append(&intitule(titre, detail));
    bloc.append(choix);
    bloc
}

fn intitule(titre: &str, detail: &str) -> gtk::Box {
    let textes = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    textes.append(&gtk::Label::builder().label(titre).xalign(0.0).build());
    textes.append(
        &gtk::Label::builder()
            .label(detail)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .css_classes(["dim-label", "caption"])
            .build(),
    );
    textes
}

fn panel_box(title: &str, subtitle: &str) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .width_request(300)
        .css_classes(["panneau"])
        .build();
    b.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    if !subtitle.is_empty() {
        b.append(
            &gtk::Label::builder()
                .label(subtitle)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["dim-label", "caption"])
                .build(),
        );
    }
    b
}

/// Ouvre ou ferme un panneau. Fermé, il ne capte plus le pointeur : le
/// réglage en fondu, surtout, garde sa taille une fois caché et volerait la
/// molette au milieu de la partition.
fn reveler(panneau: &gtk::Revealer, ouvert: bool) {
    panneau.set_reveal_child(ouvert);
    panneau.set_can_target(ouvert);
}

fn side_revealer(
    child: &gtk::Box,
    halign: gtk::Align,
    transition: gtk::RevealerTransitionType,
) -> gtk::Revealer {
    gtk::Revealer::builder()
        .child(child)
        .can_target(false)
        .halign(halign)
        .valign(gtk::Align::Fill)
        .transition_type(transition)
        .transition_duration(220)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build()
}

fn install_shortcuts(root: &adw::ApplicationWindow, sender: &ComponentSender<App>) {
    use gdk::{Key, ModifierType as M};
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let sender = sender.clone();
    keys.connect_key_pressed(move |_, key, _, state| {
        let ctrl = state.contains(M::CONTROL_MASK);
        let alt = state.contains(M::ALT_MASK);
        let msg = match key {
            Key::a | Key::A if alt => Msg::ToggleMorceaux,
            Key::e | Key::E if alt => Msg::ToggleSource,
            Key::s | Key::S if alt => Msg::Options,
            Key::n | Key::N if ctrl => Msg::Nouveau,
            Key::o | Key::O if ctrl => Msg::DemanderImport,
            Key::t | Key::T if ctrl => Msg::ToggleTheme,
            Key::plus | Key::equal | Key::KP_Add if ctrl => Msg::Zoom(Some(1.1)),
            Key::minus | Key::KP_Subtract if ctrl => Msg::Zoom(Some(1.0 / 1.1)),
            Key::_0 | Key::KP_0 if ctrl => Msg::Zoom(None),
            Key::q | Key::Q if ctrl => Msg::Quit,
            Key::h | Key::H if alt => Msg::MontrerAnnotations,
            Key::Down if alt => Msg::Defiler(1),
            Key::Up if alt => Msg::Defiler(-1),
            Key::F11 => Msg::ToggleFullscreen,
            // Échap ferme les panneaux, mais continue sa route : une boîte de
            // dialogue ouverte doit pouvoir se fermer aussi.
            Key::Escape => {
                sender.input(Msg::CloseAll);
                return glib::Propagation::Proceed;
            }
            _ => return glib::Propagation::Proceed,
        };
        sender.input(msg);
        glib::Propagation::Stop
    });
    root.add_controller(keys);
}

/// Ce qui prend la couleur d'accent : curseur, sélection, morceau ouvert,
/// interrupteurs.
fn css_accent(a: &Accent) -> String {
    format!(
        r#"
window.papier textview.source text {{ caret-color: {c}; }}
window.encre  textview.source text {{ caret-color: {s}; }}
window.papier textview text selection {{ background-color: alpha({c}, 0.22); }}
window.encre  textview text selection {{ background-color: alpha({s}, 0.25); }}
window.papier list.morceaux row.courant {{ background-color: alpha({c}, 0.16); }}
window.encre  list.morceaux row.courant {{ background-color: alpha({s}, 0.16); }}
window.papier .panneau switch:checked {{ background-color: {c}; }}
window.encre  .panneau switch:checked {{ background-color: {s}; }}
window.papier .panneau toggle-group.choix > toggle:checked {{ background-color: {c}; color: #f5f0e6; }}
window.encre  .panneau toggle-group.choix > toggle:checked {{ background-color: {s}; color: #1b1a18; }}
"#,
        c = a.clair,
        s = a.sombre,
    )
}

fn load_css(config: &Config) {
    let css = format!(
        r#"
window.vinteuil.papier, window.papier .fond {{ background-color: #f5f0e6; color: {ENCRE_PAPIER}; }}
window.vinteuil.encre,  window.encre .fond  {{ background-color: #1b1a18; color: {ENCRE_ENCRE}; }}

.pages {{ transition: opacity 300ms ease; }}
.pages.en-gravure {{ opacity: 0.55; }}
.pages:focus-visible {{ outline: none; }}
label.vide {{ font-family: "{police}", serif; font-size: 18px; opacity: 0.5; }}

textview.source, textview.source text {{ background-color: transparent; }}
textview.source {{ font-family: "{source}", monospace; font-size: {taille}px; }}
window.papier textview.source text {{ color: {ENCRE_PAPIER}; }}
window.encre  textview.source text {{ color: {ENCRE_ENCRE}; }}
window.papier label.erreur {{ color: #a4483a; }}
window.encre  label.erreur {{ color: #c46a5a; }}

.panneau {{
    padding: 18px;
    border-radius: 14px;
}}
window.papier .panneau {{ background-color: alpha(#ebe4d6, 0.97); box-shadow: 0 2px 18px alpha(black, 0.10); }}
window.encre  .panneau {{ background-color: alpha(#262421, 0.97); box-shadow: 0 2px 18px alpha(black, 0.45); }}

toast {{ border-radius: 14px; }}
window.papier toast {{ background-color: alpha(#ebe4d6, 0.97); color: {ENCRE_PAPIER}; box-shadow: 0 2px 18px alpha(black, 0.10); }}
window.encre  toast {{ background-color: alpha(#262421, 0.97); color: {ENCRE_ENCRE}; box-shadow: 0 2px 18px alpha(black, 0.45); }}

list.morceaux, list.morceaux row {{ background: transparent; }}
list.morceaux row {{ border-radius: 8px; padding: 6px 8px; font-family: "{police}", serif; }}

.panneau .reglage {{ padding: 10px 4px; }}
window.papier .panneau .reglage + .reglage {{ border-top: 1px solid alpha({ENCRE_PAPIER}, 0.08); }}
window.encre  .panneau .reglage + .reglage {{ border-top: 1px solid alpha({ENCRE_ENCRE}, 0.08); }}
window.encre  .panneau switch:checked slider {{ background-color: #1b1a18; }}

/* Le choix du thème : un rail discret, la position retenue à l'accent. */
.panneau toggle-group.choix {{ padding: 3px; border-radius: 10px; border: none; box-shadow: none; }}
window.papier .panneau toggle-group.choix {{ background-color: alpha({ENCRE_PAPIER}, 0.07); }}
window.encre  .panneau toggle-group.choix {{ background-color: alpha({ENCRE_ENCRE}, 0.07); }}
.panneau toggle-group.choix > separator {{ min-width: 0; background: none; }}
.panneau toggle-group.choix > toggle {{
    min-height: 30px;
    padding: 0 12px;
    border-radius: 7px;
    background: none;
    box-shadow: none;
    color: inherit;
    font-weight: normal;
    transition: background-color 160ms ease, color 160ms ease;
}}
.panneau toggle-group.choix > toggle:hover:not(:checked) {{ background-color: alpha(currentColor, 0.07); }}
.panneau toggle-group.choix > toggle:focus-visible {{ outline: 2px solid alpha(currentColor, 0.35); outline-offset: -2px; }}
"#,
        police = config.police,
        source = config.police_source,
        taille = config.taille_source,
    );
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("aucun écran"),
        &provider,
        // Au-dessus du thème de l'utilisateur : l'app impose son papier et son encre.
        gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
    );
}

fn texte_de(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}
