//! La partition au centre : une colonne de largeur fixe, où chaque page
//! gravée est dessinée en vectoriel (nette à tout agrandissement), à la
//! couleur du texte, papier ou encre.
//!
//! On y annote : glisser au clic gauche trace à main levée, un simple clic
//! sur une note l'annote d'un texte (`Maj` : en dessous), le clic droit gomme
//! le texte ou le trait pointé. Les annotations, traits comme textes
//! rattachés aux notes, prennent la couleur d'accent. `Alt+↓` fait défiler
//! la page toute seule, de plus en plus vite ; `Alt+↑` ralentit.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{cairo, gdk, glib};
use rsvg::prelude::*;

use crate::annotations::{self, Annotations, Trait};
use crate::gravure::{Lien, Page};
use crate::lilypond;

/// Pixels par millimètre (96 ppp, comme librsvg).
const PX_PAR_MM: f64 = 96.0 / 25.4;
/// Le défilement au clavier : une flèche, une page.
const PAS_FLECHE: f64 = 80.0;
/// Annulations retenues.
const HISTOIRE: usize = 200;
/// En deçà de cette longueur (en fraction de la largeur), un tracé n'était qu'un clic.
const TRACE_MIN: f64 = 0.004;
/// Défilement automatique : pixels par seconde, par cran ; crans au plus.
const PAS_DEFILEMENT: f64 = 12.0;
const CRANS: i32 = 12;

/// Ce que la partition demande à la fenêtre.
#[derive(Debug)]
pub enum Evenement {
    Signaler(String),
    /// Un clic gauche, sans glisser : annoter la note pointée.
    Annoter(Pointe),
    /// Un clic droit : gommer ce qui est pointé.
    Gommer(Pointe),
    /// Un trait a été posé ou effacé.
    Trace,
    Annuler,
    Refaire,
}

/// Ce qu'il y a sous un clic.
#[derive(Debug)]
pub struct Pointe {
    /// La page, où ancrer une bulle, et l'endroit du clic en pixels.
    pub zone: gtk::Widget,
    pub x: f64,
    pub y: f64,
    /// Les objets de LilyPond à portée, du plus proche au plus lointain.
    pub liens: Vec<Lien>,
    /// Le trait à portée, par son rang.
    pub trait_: Option<usize>,
    /// `Maj` était enfoncée.
    pub maj: bool,
}

pub struct Partition {
    pub scrolled: gtk::ScrolledWindow,
    clamp: adw::Clamp,
    pages: gtk::Box,
    vide: gtk::Label,
    /// Largeur de la colonne à l'agrandissement 1, en pixels.
    largeur: f64,
    defilement: adw::TimedAnimation,
    /// Le cran du défilement automatique : positif vers le bas, nul à l'arrêt.
    vitesse: Rc<Cell<i32>>,
    etat: Rc<Etat>,
}

/// Ce que partagent les pages et leurs gestes.
struct Etat {
    /// Les pages affichées, et le SVG que chacune dessine.
    dessins: RefCell<Vec<(gtk::DrawingArea, rsvg::Handle)>>,
    /// Les liens de chaque page affichée.
    liens: RefCell<Vec<Vec<Lien>>>,
    /// Hauteur relative de chaque page affichée.
    proportions: RefCell<Vec<f64>>,
    annotations: RefCell<Annotations>,
    passe: RefCell<Vec<Annotations>>,
    futur: RefCell<Vec<Annotations>>,
    /// Le fichier des traits du morceau ouvert.
    fichier: RefCell<Option<PathBuf>>,
    /// Les proportions déjà signalées comme décalées : on ne le redit pas à chaque gravure.
    deja_signale: RefCell<Vec<f64>>,
    /// Le trait qu'on est en train de tracer.
    en_cours: RefCell<Option<Trait>>,
    visibles: Cell<bool>,
    /// La couleur de la musique et celle des annotations, `#rrggbb`.
    encre: RefCell<String>,
    accent: RefCell<String>,
    rappel: Box<dyn Fn(Evenement)>,
}

impl Etat {
    fn redessiner(&self) {
        for (area, _) in self.dessins.borrow().iter() {
            area.queue_draw();
        }
    }

    fn recolorer(&self) {
        let feuille = feuille(
            &self.encre.borrow(),
            &self.accent.borrow(),
            self.visibles.get(),
        );
        for (area, handle) in self.dessins.borrow().iter() {
            colorer(handle, &feuille);
            area.queue_draw();
        }
    }

    /// Change les traits, en gardant de quoi annuler, et les enregistre.
    fn modifier(&self, f: impl FnOnce(&mut Annotations)) {
        let avant = self.annotations.borrow().clone();
        f(&mut self.annotations.borrow_mut());
        if *self.annotations.borrow() == avant {
            return;
        }
        let mut passe = self.passe.borrow_mut();
        passe.push(avant);
        if passe.len() > HISTOIRE {
            passe.remove(0);
        }
        drop(passe);
        self.futur.borrow_mut().clear();
        self.enregistrer();
        (self.rappel)(Evenement::Trace);
    }

    fn enregistrer(&self) {
        self.redessiner();
        let Some(fichier) = self.fichier.borrow().clone() else {
            return;
        };
        let mut a = self.annotations.borrow_mut();
        // Annoter, c'est accepter la mise en page telle qu'elle est.
        a.proportions = self.proportions.borrow().clone();
        if let Err(e) = a.ecrire(&fichier) {
            (self.rappel)(Evenement::Signaler(format!(
                "Annotations non enregistrées : {e:#}"
            )));
        }
    }
}

impl Partition {
    /// `largeur` : celle de la page gravée, en millimètres ; `rappel` reçoit
    /// ce que la partition demande à la fenêtre.
    pub fn new(largeur: f64, rappel: impl Fn(Evenement) + 'static) -> Self {
        let vide = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .vexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["vide"])
            .build();
        let pages = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(64)
            .margin_top(56)
            .margin_bottom(96)
            .focusable(true)
            .css_classes(["pages"])
            .build();
        pages.append(&vide);

        let clamp = adw::Clamp::builder().child(&pages).build();
        // Viewport explicite : sans `scroll_to_focus`, la page ne saute pas
        // quand la partition reprend le focus.
        let viewport = gtk::Viewport::builder()
            .scroll_to_focus(false)
            .child(&clamp)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&viewport)
            .build();

        let defilement = adw::TimedAnimation::new(
            &scrolled,
            0.0,
            0.0,
            160,
            adw::PropertyAnimationTarget::new(&scrolled.vadjustment(), "value"),
        );
        defilement.set_easing(adw::Easing::EaseOutCubic);

        let etat = Rc::new(Etat {
            dessins: RefCell::default(),
            liens: RefCell::default(),
            proportions: RefCell::default(),
            annotations: RefCell::default(),
            passe: RefCell::default(),
            futur: RefCell::default(),
            fichier: RefCell::default(),
            deja_signale: RefCell::default(),
            en_cours: RefCell::default(),
            visibles: Cell::new(true),
            encre: RefCell::new("#2a2521".into()),
            accent: RefCell::new("#8a5a2b".into()),
            rappel: Box::new(rappel),
        });

        let partition = Self {
            scrolled,
            clamp,
            pages,
            vide,
            largeur: largeur * PX_PAR_MM,
            defilement,
            vitesse: Rc::default(),
            etat,
        };
        partition.clavier();
        partition
    }

    /// Les flèches, les pages, le début et la fin, en douceur ; l'annulation.
    fn clavier(&self) {
        let keys = gtk::EventControllerKey::new();
        let (scrolled, animation, etat) = (
            self.scrolled.clone(),
            self.defilement.clone(),
            Rc::downgrade(&self.etat),
        );
        keys.connect_key_pressed(move |_, key, _, state| {
            use gdk::ModifierType as M;
            if state.contains(M::CONTROL_MASK) {
                let Some(etat) = etat.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let evenement = match key {
                    gdk::Key::z if !state.contains(M::SHIFT_MASK) => Evenement::Annuler,
                    gdk::Key::Z | gdk::Key::z | gdk::Key::y => Evenement::Refaire,
                    _ => return glib::Propagation::Proceed,
                };
                (etat.rappel)(evenement);
                return glib::Propagation::Stop;
            }
            if !state.is_empty() && state != M::SHIFT_MASK {
                return glib::Propagation::Proceed;
            }
            let adj = scrolled.vadjustment();
            let page = adj.page_size() * 0.85;
            // Depuis la cible en cours : des appuis rapprochés s'additionnent.
            let depart = if animation.state() == adw::AnimationState::Playing {
                animation.value_to()
            } else {
                adj.value()
            };
            let cible = match key {
                gdk::Key::Down | gdk::Key::j => depart + PAS_FLECHE,
                gdk::Key::Up | gdk::Key::k => depart - PAS_FLECHE,
                gdk::Key::Page_Down | gdk::Key::space => depart + page,
                gdk::Key::Page_Up => depart - page,
                gdk::Key::Home => adj.lower(),
                gdk::Key::End => adj.upper(),
                _ => return glib::Propagation::Proceed,
            };
            let cible = cible.clamp(
                adj.lower(),
                (adj.upper() - adj.page_size()).max(adj.lower()),
            );
            animation.set_value_from(adj.value());
            animation.set_value_to(cible);
            animation.reset();
            animation.play();
            glib::Propagation::Stop
        });
        self.scrolled.add_controller(keys);
    }

    pub fn grab_focus(&self) {
        self.pages.grab_focus();
    }

    /// Le message affiché quand il n'y a rien à montrer.
    pub fn vide(&self, message: &str) {
        self.remplacer(Vec::new());
        self.etat.proportions.borrow_mut().clear();
        self.etat.liens.borrow_mut().clear();
        self.vide.set_label(message);
        self.vide.set_visible(true);
    }

    /// Aucune page n'est affichée.
    pub fn est_vide(&self) -> bool {
        self.etat.dessins.borrow().is_empty()
    }

    /// Montre les pages gravées.
    pub fn afficher(&self, pages: Vec<Page>) -> Result<(), glib::Error> {
        let feuille = feuille(
            &self.etat.encre.borrow(),
            &self.etat.accent.borrow(),
            self.etat.visibles.get(),
        );
        let mut dessins = Vec::with_capacity(pages.len());
        let mut proportions = Vec::with_capacity(pages.len());
        let mut liens = Vec::with_capacity(pages.len());
        for (i, p) in pages.into_iter().enumerate() {
            let handle = rsvg::Handle::from_data(p.svg.as_bytes())?;
            colorer(&handle, &feuille);
            let (l, h) = handle.intrinsic_size_in_pixels().unwrap_or((1.0, 1.0));
            let ratio = if l > 0.0 { h / l } else { 1.0 };
            proportions.push(ratio);
            liens.push(p.liens);
            dessins.push((page(&handle, i, ratio, &self.etat), handle));
        }
        self.vide.set_visible(false);
        self.remplacer(dessins);
        self.etat.proportions.replace(proportions);
        self.etat.liens.replace(liens);
        self.verifier_decalage();
        Ok(())
    }

    fn remplacer(&self, dessins: Vec<(gtk::DrawingArea, rsvg::Handle)>) {
        for (area, _) in self.etat.dessins.borrow().iter() {
            self.pages.remove(area);
        }
        for (area, _) in &dessins {
            self.pages.append(area);
        }
        *self.etat.dessins.borrow_mut() = dessins;
    }

    /// Prévient si les traits ont été posés sur une autre mise en page.
    fn verifier_decalage(&self) {
        let proportions = self.etat.proportions.borrow().clone();
        if self.etat.annotations.borrow().decalees(&proportions)
            && *self.etat.deja_signale.borrow() != proportions
        {
            (self.etat.rappel)(Evenement::Signaler(
                "La mise en page a changé : les traits peuvent être décalés".into(),
            ));
            self.etat.deja_signale.replace(proportions);
        }
    }

    /// Les traits du morceau, qu'on vient d'ouvrir.
    pub fn charger_annotations(&self, morceau: &Path) {
        let fichier = annotations::chemin(morceau);
        let a = Annotations::lire(&fichier).unwrap_or_else(|e| {
            (self.etat.rappel)(Evenement::Signaler(format!(
                "Annotations illisibles : {e:#}"
            )));
            Annotations::default()
        });
        self.etat.annotations.replace(a);
        self.etat.fichier.replace(Some(fichier));
        self.etat.passe.borrow_mut().clear();
        self.etat.futur.borrow_mut().clear();
        self.etat.deja_signale.borrow_mut().clear();
        self.etat.redessiner();
    }

    /// `annuler` : revient sur le dernier changement des traits ; sinon le refait.
    pub fn remonter(&self, annuler: bool) {
        let etat = &self.etat;
        let (depuis, vers) = if annuler {
            (&etat.passe, &etat.futur)
        } else {
            (&etat.futur, &etat.passe)
        };
        let Some(precedent) = depuis.borrow_mut().pop() else {
            return;
        };
        vers.borrow_mut().push(etat.annotations.replace(precedent));
        etat.enregistrer();
    }

    pub fn effacer_trait(&self, rang: usize) {
        self.etat.modifier(|a| {
            if rang < a.traits.len() {
                a.traits.remove(rang);
            }
        });
    }

    /// Montre ou masque les annotations ; rend vrai si elles sont visibles.
    pub fn basculer_annotations(&self) -> bool {
        let visibles = !self.etat.visibles.get();
        self.etat.visibles.set(visibles);
        self.etat.recolorer();
        visibles
    }

    /// La couleur de la musique (le thème) et celle des annotations (l'accent).
    pub fn couleurs(&self, encre: &str, accent: &str) {
        self.etat.encre.replace(encre.to_string());
        self.etat.accent.replace(accent.to_string());
        self.etat.recolorer();
    }

    /// Le défilement automatique, un cran plus bas (`1`) ou plus haut (`-1`) :
    /// chaque appui accélère, à l'envers il ralentit puis repart dans l'autre
    /// sens. Rend le cran atteint.
    pub fn defiler(&self, sens: i32) -> i32 {
        let avant = self.vitesse.get();
        let cran = (avant + sens).clamp(-CRANS, CRANS);
        self.vitesse.set(cran);
        if avant == 0 && cran != 0 {
            self.lancer_defilement();
        }
        cran
    }

    pub fn arreter_defilement(&self) {
        self.vitesse.set(0);
    }

    /// À chaque image : avancer selon le temps écoulé, jusqu'au bout de la page.
    fn lancer_defilement(&self) {
        let vitesse = self.vitesse.clone();
        let precedente = Cell::new(None::<i64>);
        self.scrolled.add_tick_callback(move |scrolled, horloge| {
            let cran = vitesse.get();
            if cran == 0 {
                return glib::ControlFlow::Break;
            }
            let maintenant = horloge.frame_time();
            let ecoule = precedente
                .replace(Some(maintenant))
                .map_or(0.0, |p| (maintenant - p) as f64 / 1e6);
            let adj = scrolled.vadjustment();
            let fin = (adj.upper() - adj.page_size()).max(adj.lower());
            let valeur =
                (adj.value() + cran as f64 * PAS_DEFILEMENT * ecoule).clamp(adj.lower(), fin);
            adj.set_value(valeur);
            if (cran > 0 && valeur >= fin) || (cran < 0 && valeur <= adj.lower()) {
                vitesse.set(0);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn zoomer(&self, zoom: f64) {
        // Toute la largeur voulue dès qu'il y a la place, sans resserrement progressif.
        let largeur = (self.largeur * zoom).round() as i32;
        self.clamp.set_maximum_size(largeur);
        self.clamp.set_tightening_threshold(largeur);
    }

    /// Estompée pendant la gravure : ce qu'on voit est l'état précédent.
    pub fn en_gravure(&self, oui: bool) {
        if oui {
            self.pages.add_css_class("en-gravure");
        } else {
            self.pages.remove_css_class("en-gravure");
        }
    }
}

/// La feuille de style des pages : le noir de LilyPond (`currentColor`) prend
/// la couleur du thème, la couleur repère des annotations celle de l'accent.
fn feuille(encre: &str, accent: &str, visibles: bool) -> String {
    let repere = lilypond::REPERE;
    let annotations = if visibles {
        format!("fill: {accent}")
    } else {
        "fill-opacity: 0".into()
    };
    format!("svg {{ color: {encre}; }}\n[fill=\"{repere}\"] {{ {annotations}; }}\n")
}

fn colorer(handle: &rsvg::Handle, feuille: &str) {
    if let Err(e) = handle.set_stylesheet(feuille.as_bytes()) {
        eprintln!("vinteuil : couleur de la partition ({e})");
    }
}

/// `#rrggbb` en composantes de 0 à 1.
fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let hex = hex.strip_prefix('#')?;
    let c = |i: usize| Some(u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()? as f64 / 255.0);
    Some((c(0)?, c(2)?, c(4)?))
}

/// L'épaisseur relative du trait : la pression du stylet, s'il y en a un.
fn pression(geste: &impl IsA<gtk::EventController>) -> f64 {
    geste
        .current_event()
        .and_then(|e| e.axis(gdk::AxisUse::Pressure))
        .filter(|p| *p > 0.0)
        .map_or(1.0, |p| 0.35 + 1.3 * p)
}

/// Distance d'un point à un cadre (nulle à l'intérieur).
fn distance(cadre: &[f64; 4], x: f64, y: f64) -> f64 {
    let dx = (cadre[0] - x).max(x - cadre[2]).max(0.0);
    let dy = (cadre[1] - y).max(y - cadre[3]).max(0.0);
    dx.hypot(dy)
}

/// Une page : toute la largeur de la colonne, la hauteur qui en découle, et
/// par-dessus ses traits.
fn page(handle: &rsvg::Handle, index: usize, ratio: f64, etat: &Rc<Etat>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().hexpand(true).build();
    area.set_cursor_from_name(Some("crosshair"));
    let (dessin, faible) = (handle.clone(), Rc::downgrade(etat));
    area.set_draw_func(move |_, cr, l, h| {
        let cadre = rsvg::Rectangle::new(0.0, 0.0, l as f64, h as f64);
        if let Err(e) = dessin.render_document(cr, &cadre) {
            eprintln!("vinteuil : dessin de la partition ({e})");
        }
        if let Some(etat) = faible.upgrade() {
            dessiner_traits(&etat, index, cr, l as f64);
        }
    });
    // La hauteur suit la largeur : la page garde ses proportions. Après
    // l'allocation en cours : une taille demandée pendant celle-ci serait perdue.
    area.connect_resize(move |area, l, _| {
        let voulue = (l as f64 * ratio).round() as i32;
        if area.content_height() != voulue {
            let area = area.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(area) = area.upgrade() {
                    area.set_content_height(voulue);
                }
            });
        }
    });
    gestes(&area, index, Rc::downgrade(etat));
    area
}

/// Le clic gauche trace, ou annote la note s'il ne glisse pas ; le clic droit gomme.
fn gestes(area: &gtk::DrawingArea, index: usize, etat: Weak<Etat>) {
    let drag = gtk::GestureDrag::builder()
        .button(gdk::BUTTON_PRIMARY)
        .build();
    drag.connect_drag_begin(glib::clone!(
        #[strong]
        etat,
        move |g, x, y| {
            let (Some(etat), Some(area)) = (etat.upgrade(), g.widget()) else {
                return;
            };
            g.set_state(gtk::EventSequenceState::Claimed);
            // La partition prend le focus : clavier et annulation s'y adressent.
            if let Some(pages) = area.parent() {
                pages.grab_focus();
            }
            let l = area.width() as f64;
            etat.en_cours.replace(Some(Trait {
                page: index,
                points: vec![[x / l, y / l, pression(g)]],
            }));
        }
    ));
    drag.connect_drag_update(glib::clone!(
        #[strong]
        etat,
        move |g, dx, dy| {
            let (Some(etat), Some(area), Some((x, y))) =
                (etat.upgrade(), g.widget(), g.start_point())
            else {
                return;
            };
            let l = area.width() as f64;
            if let Some(t) = etat.en_cours.borrow_mut().as_mut() {
                t.points.push([(x + dx) / l, (y + dy) / l, pression(g)]);
            }
            area.queue_draw();
        }
    ));
    drag.connect_drag_end(glib::clone!(
        #[strong]
        etat,
        move |g, _, _| {
            let Some(etat) = etat.upgrade() else { return };
            let Some(t) = etat.en_cours.take() else {
                return;
            };
            let longueur: f64 = t
                .points
                .windows(2)
                .map(|p| (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]))
                .sum();
            if longueur >= TRACE_MIN {
                // Annotées, les pages se montrent.
                if !etat.visibles.get() {
                    etat.visibles.set(true);
                    etat.recolorer();
                }
                etat.modifier(|a| a.traits.push(t));
            } else if let (Some(area), Some((x, y))) = (g.widget(), g.start_point()) {
                // Un simple clic : on annote la note pointée.
                area.queue_draw();
                let maj = g
                    .current_event_state()
                    .contains(gdk::ModifierType::SHIFT_MASK);
                (etat.rappel)(Evenement::Annoter(pointe(&etat, index, area, x, y, maj)));
            }
        }
    ));
    area.add_controller(drag);

    let clic = gtk::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    clic.connect_pressed(move |g, _, x, y| {
        let (Some(etat), Some(area)) = (etat.upgrade(), g.widget()) else {
            return;
        };
        (etat.rappel)(Evenement::Gommer(pointe(&etat, index, area, x, y, false)));
    });
    area.add_controller(clic);
}

/// Ce qu'il y a autour de (x, y), en pixels, sur la page `index`.
fn pointe(etat: &Etat, index: usize, zone: gtk::Widget, x: f64, y: f64, maj: bool) -> Pointe {
    let l = zone.width() as f64;
    let (px, py) = (x / l, y / l);
    let mut liens: Vec<(f64, Lien)> = etat
        .liens
        .borrow()
        .get(index)
        .into_iter()
        .flatten()
        .map(|lien| (distance(&lien.cadre, px, py), lien.clone()))
        .filter(|(d, _)| *d <= annotations::PRISE * 1.5)
        .collect();
    liens.sort_by(|a, b| a.0.total_cmp(&b.0));
    let trait_ = etat.annotations.borrow().trait_proche(index, px, py);
    Pointe {
        zone,
        x,
        y,
        liens: liens.into_iter().map(|(_, l)| l).collect(),
        trait_,
        maj,
    }
}

/// Un petit champ à l'endroit cliqué ; `Entrée` le valide.
pub fn bulle_texte(
    zone: &gtk::Widget,
    x: f64,
    y: f64,
    indication: &str,
    valider: impl Fn(String) + 'static,
) {
    let entree = gtk::Entry::builder()
        .placeholder_text(indication)
        .width_chars(24)
        .build();
    let bulle = bulle(zone, x, y, &entree);
    entree.connect_activate(glib::clone!(
        #[weak]
        bulle,
        move |e| {
            let texte = e.text().trim().to_string();
            bulle.popdown();
            if !texte.is_empty() {
                valider(texte);
            }
        }
    ));
    bulle.popup();
    entree.grab_focus();
}

fn bulle(zone: &gtk::Widget, x: f64, y: f64, contenu: &impl IsA<gtk::Widget>) -> gtk::Popover {
    let bulle = gtk::Popover::builder()
        .child(contenu)
        .pointing_to(&gdk::Rectangle::new(x as i32, y as i32, 1, 1))
        .position(gtk::PositionType::Bottom)
        .has_arrow(false)
        .css_classes(["bulle"])
        .build();
    bulle.set_parent(zone);
    // Détachée après coup : pas pendant le signal qui la ferme.
    bulle.connect_closed(|b| {
        let b = b.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(b) = b.upgrade() {
                b.unparent();
            }
        });
    });
    bulle
}

fn dessiner_traits(etat: &Etat, index: usize, cr: &cairo::Context, l: f64) {
    if !etat.visibles.get() && etat.en_cours.borrow().is_none() {
        return;
    }
    let Some((r, g, b)) = rgb(&etat.accent.borrow()) else {
        return;
    };
    cr.set_source_rgb(r, g, b);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    let a = etat.annotations.borrow();
    let en_cours = etat.en_cours.borrow();
    let traits = a.traits.iter().filter(|_| etat.visibles.get());
    for t in traits.chain(en_cours.iter()).filter(|t| t.page == index) {
        tracer(cr, t, l);
    }
}

/// Un trait, segment par segment : l'épaisseur suit la pression.
fn tracer(cr: &cairo::Context, t: &Trait, l: f64) {
    let epaisseur = |p: f64| annotations::EPAISSEUR * l * p;
    if let [p] = t.points.as_slice() {
        cr.arc(
            p[0] * l,
            p[1] * l,
            epaisseur(p[2]) / 2.0,
            0.0,
            std::f64::consts::TAU,
        );
        let _ = cr.fill();
        return;
    }
    for paire in t.points.windows(2) {
        let [a, b] = [paire[0], paire[1]];
        cr.set_line_width(epaisseur((a[2] + b[2]) / 2.0));
        cr.move_to(a[0] * l, a[1] * l);
        cr.line_to(b[0] * l, b[1] * l);
        let _ = cr.stroke();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une note au centre d'une page blanche, comme la sort LilyPond, et à
    /// gauche une annotation à la couleur repère.
    fn svg() -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm" viewBox="0 0 20 10">
<rect x="5" y="2" width="10" height="6" fill="currentColor"/><rect x="0" y="0" width="3" height="3" fill="{}"/></svg>"#,
            lilypond::REPERE
        )
    }

    /// Les pixels de la note et de l'annotation.
    fn pixels(feuille: &str) -> [[u8; 4]; 2] {
        let handle = rsvg::Handle::from_data(svg().as_bytes()).unwrap();
        colorer(&handle, feuille);
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 80, 40).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            handle
                .render_document(&cr, &rsvg::Rectangle::new(0.0, 0.0, 80.0, 40.0))
                .unwrap();
        }
        let data = surface.data().unwrap();
        // ARGB32 en mémoire, petit-boutiste : B, G, R, A.
        let px = |x: usize, y: usize| {
            let i = (y * 80 + x) * 4;
            [data[i + 2], data[i + 1], data[i], data[i + 3]]
        };
        [px(40, 20), px(4, 4)]
    }

    #[test]
    fn la_musique_prend_l_encre_et_les_annotations_l_accent() {
        assert_eq!(
            pixels(&feuille("#d9d2c5", "#d4a35f", true)),
            [[0xd9, 0xd2, 0xc5, 0xff], [0xd4, 0xa3, 0x5f, 0xff]]
        );
        assert_eq!(
            pixels(&feuille("#2a2521", "#8a5a2b", true))[0],
            [0x2a, 0x25, 0x21, 0xff]
        );
        // Masquées, les annotations disparaissent ; la musique reste.
        let [note, annotation] = pixels(&feuille("#2a2521", "#8a5a2b", false));
        assert_eq!((note[3], annotation[3]), (0xff, 0));
    }

    #[test]
    fn couleurs_hexadecimales() {
        assert_eq!(rgb("#ff0080"), Some((1.0, 0.0, 128.0 / 255.0)));
        assert_eq!(rgb("ff0080"), None);
        assert_eq!(rgb("#ff00"), None);
    }

    #[test]
    fn distance_a_un_cadre() {
        let c = [0.1, 0.1, 0.2, 0.2];
        assert_eq!(distance(&c, 0.15, 0.15), 0.0);
        assert!((distance(&c, 0.25, 0.15) - 0.05).abs() < 1e-12);
    }
}
