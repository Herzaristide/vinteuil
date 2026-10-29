//! Les opérations sur le source LilyPond qu'on lance depuis la partition :
//! annoter une note (`c4^\annotation "pizz."`), retirer une annotation.
//!
//! Les positions sont celles des liens de LilyPond : une ligne (à partir de 0
//! ici) et une colonne en caractères, comme celles d'un tampon GTK. Les
//! opérations rendent des éditions, à appliquer telles quelles au tampon :
//! elles sont rangées de la fin vers le début du texte, si bien qu'aucune ne
//! déplace la suivante.

/// La fonction que Vinteuil ajoute en tête du morceau à la première annotation.
/// Sa couleur sert de repère : Vinteuil la remplace par l'accent à
/// l'affichage ; ailleurs, c'est un noir à peine teinté.
pub const DEFINITION: &str = "\
% Vinteuil : \\annotation, un texte rattaché à une note (c4^\\annotation \"pizz.\").
annotation = #(define-event-function (texte) (markup?)
  #{ -\\tweak color #(rgb-color 0.1 0.1 0.11) -\\markup \\italic #texte #})
";

/// La couleur de la définition, telle que l'écrit le moteur Cairo.
pub const REPERE: &str = "rgb(10%, 10%, 11%)";

/// Le début de la définition, pour savoir si le morceau l'a déjà.
const SIGNATURE: &str = "annotation = #(define-event-function";
const NOM: &str = "\\annotation";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edition {
    Inserer {
        ligne: usize,
        colonne: usize,
        texte: String,
    },
    Supprimer {
        ligne: usize,
        colonne: usize,
        longueur: usize,
    },
}

/// Ce qui se trouve à la position d'un lien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cible {
    /// Une note, un accord ou un silence : on peut l'annoter.
    Note,
    /// Une annotation de Vinteuil, et son texte.
    Annotation(String),
    Autre,
}

pub fn cible(source: &str, ligne: usize, colonne: usize) -> Cible {
    let Some(texte) = source.lines().nth(ligne) else {
        return Cible::Autre;
    };
    let chars: Vec<char> = texte.chars().collect();
    if let Some((texte, _)) = annotation_a(&chars, colonne) {
        return Cible::Annotation(texte);
    }
    match fin_de_note(&chars, colonne) {
        Some(_) => Cible::Note,
        None => Cible::Autre,
    }
}

/// Les éditions qui attachent `texte` à la note qui commence à cette position.
pub fn annoter(
    source: &str,
    ligne: usize,
    colonne: usize,
    dessous: bool,
    texte: &str,
) -> Option<Vec<Edition>> {
    let chars: Vec<char> = source.lines().nth(ligne)?.chars().collect();
    let fin = fin_de_note(&chars, colonne)?;
    let direction = if dessous { '_' } else { '^' };
    let mut editions = vec![Edition::Inserer {
        ligne,
        colonne: fin,
        texte: format!("{direction}{NOM} {}", chaine(texte)),
    }];
    if !source.contains(SIGNATURE) {
        // Juste après `\version`, ou tout en haut.
        let version = source
            .lines()
            .position(|l| l.trim_start().starts_with("\\version"));
        let edition = match version {
            Some(v) => Edition::Inserer {
                ligne: v + 1,
                colonne: 0,
                texte: format!("\n{DEFINITION}"),
            },
            None => Edition::Inserer {
                ligne: 0,
                colonne: 0,
                texte: format!("{DEFINITION}\n"),
            },
        };
        editions.push(edition);
    }
    editions.sort_by_key(|e| std::cmp::Reverse(position(e)));
    Some(editions)
}

/// Les éditions qui retirent l'annotation qui commence à cette position.
pub fn retirer(source: &str, ligne: usize, colonne: usize) -> Option<Vec<Edition>> {
    let chars: Vec<char> = source.lines().nth(ligne)?.chars().collect();
    let (_, longueur) = annotation_a(&chars, colonne)?;
    Some(vec![Edition::Supprimer {
        ligne,
        colonne,
        longueur,
    }])
}

fn position(e: &Edition) -> (usize, usize) {
    match e {
        Edition::Inserer { ligne, colonne, .. } | Edition::Supprimer { ligne, colonne, .. } => {
            (*ligne, *colonne)
        }
    }
}

/// Applique des éditions à un texte (comme on les applique au tampon).
#[cfg(test)]
pub fn appliquer(source: &str, editions: &[Edition]) -> String {
    let mut chars: Vec<char> = source.chars().collect();
    for e in editions {
        let (ligne, colonne) = position(e);
        let texte: String = chars.iter().collect();
        let debut: usize = texte
            .split('\n')
            .take(ligne)
            .map(|l| l.chars().count() + 1)
            .sum::<usize>()
            + colonne;
        match e {
            Edition::Inserer { texte, .. } => {
                chars.splice(debut..debut, texte.chars());
            }
            Edition::Supprimer { longueur, .. } => {
                chars.drain(debut..debut + longueur);
            }
        }
    }
    chars.into_iter().collect()
}

/// Une chaîne LilyPond entre guillemets.
fn chaine(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `^\annotation "texte"` (ou `_`, `-`, ou sans direction) à cette position :
/// le texte, et la longueur du tout.
fn annotation_a(chars: &[char], colonne: usize) -> Option<(String, usize)> {
    let mut i = colonne;
    if matches!(chars.get(i), Some('^' | '_' | '-')) {
        i += 1;
    }
    let nom: Vec<char> = NOM.chars().collect();
    if chars.get(i..i + nom.len())? != nom.as_slice() {
        return None;
    }
    i += nom.len();
    // Pas `\annotationBis`.
    if chars.get(i).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    let mut texte = String::new();
    loop {
        match chars.get(i)? {
            '"' => break,
            '\\' => {
                i += 1;
                texte.push(*chars.get(i)?);
            }
            c => texte.push(*c),
        }
        i += 1;
    }
    Some((texte, i + 1 - colonne))
}

/// Où finit la note (nom, octave, altération, durée) qui commence à
/// `colonne`. Une note d'accord renvoie à la fin de l'accord : c'est
/// l'accord qu'on annote.
fn fin_de_note(chars: &[char], colonne: usize) -> Option<usize> {
    let n = chars.len();
    let mut i = if let Some(fin) = fin_d_accord(chars, colonne) {
        fin
    } else {
        if !chars.get(colonne)?.is_ascii_alphabetic() {
            return None;
        }
        let mut i = colonne;
        while i < n && chars[i].is_ascii_alphabetic() {
            i += 1;
        }
        while i < n && matches!(chars[i], '\'' | ',' | '!' | '?') {
            i += 1;
        }
        i
    };
    while i < n && chars[i].is_ascii_digit() {
        i += 1;
    }
    while i < n && chars[i] == '.' {
        i += 1;
    }
    // `R1*4`, `c4*2/3`.
    if i < n && chars[i] == '*' {
        i += 1;
        while i < n && (chars[i].is_ascii_digit() || chars[i] == '/') {
            i += 1;
        }
    }
    Some(i)
}

/// Si `colonne` est dans un accord `<…>` (ou sur son `<`), la position juste
/// après son `>`. `<<` et `>>` ne sont pas des accords.
fn fin_d_accord(chars: &[char], colonne: usize) -> Option<usize> {
    let simple = |i: usize, c: char| {
        chars.get(i) == Some(&c) && chars.get(i + 1) != Some(&c) && (i == 0 || chars[i - 1] != c)
    };
    // L'ouverture, en remontant sans croiser de fermeture.
    let ouverture = (0..=colonne.min(chars.len().saturating_sub(1)))
        .rev()
        .take_while(|&i| i == colonne || !simple(i, '>'))
        .find(|&i| simple(i, '<'))?;
    let fermeture = (ouverture + 1..chars.len()).find(|&i| simple(i, '>'))?;
    (fermeture >= colonne).then_some(fermeture + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MORCEAU: &str =
        "\\version \"2.24.0\"\n\\relative c' {\n  c4 d8. e16 <c e g>2 | r4 fis'!2 R1*4\n}\n";

    fn col(source: &str, ligne: usize, motif: &str) -> usize {
        let l = source.lines().nth(ligne).unwrap();
        l[..l.find(motif).unwrap()].chars().count()
    }

    #[test]
    fn annoter_une_note_ajoute_la_definition_une_fois() {
        let e = annoter(MORCEAU, 2, col(MORCEAU, 2, "d8."), false, "4e doigt").unwrap();
        let s = appliquer(MORCEAU, &e);
        assert!(s.contains("d8.^\\annotation \"4e doigt\" e16"), "{s}");
        // La définition vient juste après `\version`.
        assert!(
            s.starts_with("\\version \"2.24.0\"\n\n% Vinteuil : \\annotation"),
            "{s}"
        );
        assert_eq!(s.matches(SIGNATURE).count(), 1);

        // Une seconde annotation, en dessous : pas de seconde définition.
        let ligne = s.lines().position(|l| l.contains("fis'!2")).unwrap();
        let e = annoter(&s, ligne, col(&s, ligne, "fis"), true, "p").unwrap();
        assert_eq!(e.len(), 1);
        let s = appliquer(&s, &e);
        assert!(s.contains("fis'!2_\\annotation \"p\" R1*4"), "{s}");
    }

    #[test]
    fn annoter_un_accord_et_un_silence() {
        // Une note de l'accord : c'est l'accord qu'on annote.
        let e = annoter(MORCEAU, 2, col(MORCEAU, 2, "e g>"), false, "arp.").unwrap();
        assert!(appliquer(MORCEAU, &e).contains("<c e g>2^\\annotation \"arp.\" |"));
        let e = annoter(MORCEAU, 2, col(MORCEAU, 2, "R1"), false, "tacet").unwrap();
        assert!(appliquer(MORCEAU, &e).contains("R1*4^\\annotation \"tacet\"\n"));
        // `<<` n'est pas un accord, ni `|` une note.
        assert_eq!(
            fin_d_accord(&"<< c4 >>".chars().collect::<Vec<_>>(), 3),
            None
        );
        assert_eq!(annoter(MORCEAU, 2, col(MORCEAU, 2, "|"), false, "x"), None);
    }

    #[test]
    fn guillemets_et_accents() {
        let source = "\\relative { é4 c4 }";
        let e = annoter(source, 0, col(source, 0, "c4"), false, "le \"bon\" doigt").unwrap();
        let s = appliquer(source, &e);
        assert!(
            s.ends_with("c4^\\annotation \"le \\\"bon\\\" doigt\" }"),
            "{s}"
        );
        // Sans `\version`, la définition va tout en haut.
        assert!(s.starts_with("% Vinteuil"));
    }

    #[test]
    fn reconnaitre_et_retirer() {
        let source = "{ c4^\\annotation \"pizz.\" d4 e4\\annotation \"x\" f4 }";
        let a = col(source, 0, "^\\annotation");
        assert_eq!(cible(source, 0, a), Cible::Annotation("pizz.".into()));
        assert_eq!(cible(source, 0, col(source, 0, "d4")), Cible::Note);
        assert_eq!(cible(source, 0, 0), Cible::Autre);
        let s = appliquer(source, &retirer(source, 0, a).unwrap());
        assert_eq!(s, "{ c4 d4 e4\\annotation \"x\" f4 }");
        // Sans direction.
        let s = appliquer(&s, &retirer(&s, 0, col(&s, 0, "\\annotation")).unwrap());
        assert_eq!(s, "{ c4 d4 e4 f4 }");
        assert_eq!(retirer(&s, 0, 2), None);
    }
}
