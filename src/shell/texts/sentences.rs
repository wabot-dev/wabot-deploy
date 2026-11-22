//! The texts that are computed, because they carry a number or a name.
//!
//! **A sentence with a figure in it cannot be a table entry.** The
//! figure goes in a different place in different languages, and a
//! `format!` at the call site fixes the English word order for
//! everybody. Building the sentence here is what lets the Spanish put
//! the number where Spanish wants it.
//!
//! Each of these is one function, and the two languages are two arms of
//! one `match` — so a translation that forgets the number is a compile
//! error rather than a page with a hole in it.

use super::language::current;
use crate::domain::values::language::Language;

/// What a drain did, and what it could not do.
pub fn copies_moved(moved: usize, refused: &[String]) -> String {
    match current() {
        Language::En => match refused.is_empty() {
            true => format!("{moved} copies moved."),
            false => format!("{moved} copies moved; {}", refused.join("; ")),
        },
        Language::Es => match refused.is_empty() {
            true => format!("Se movieron {moved} copias."),
            false => format!("Se movieron {moved} copias; {}", refused.join("; ")),
        },
    }
}

/// Why a node cannot stop owning things yet.
pub fn still_owns_projects(count: usize) -> String {
    match current() {
        Language::En => {
            format!("this node owns {count} project(s) of its own — move or delete them first")
        }
        Language::Es => {
            format!("este nodo tiene {count} proyecto(s) propios — muévelos o elimínalos primero")
        }
    }
}

/// Services this organisation runs with no memory ceiling at all.
///
/// **A quota demands ceilings.** One unmeasurable service makes the
/// whole quota decoration, so the page says how many rather than leaving
/// somebody to meet the refusal later and go looking.
pub fn services_without_a_ceiling(count: usize) -> String {
    match current() {
        Language::En => format!(
            "{count} service(s) have no memory ceiling, so they count for nothing here — \
             a quota with one of those under it is not a ceiling"
        ),
        Language::Es => format!(
            "{count} servicio(s) no tienen techo de memoria, así que no cuentan aquí — \
             una cuota con uno de ésos debajo no es un techo"
        ),
    }
}

/// Who an invitation into an organisation is for, and what it makes
/// them.
///
/// A sentence rather than a `t()` fragment plus a name: the name sits in
/// a different place in each language, and a `format!` at the call site
/// would fix the English word order for everybody.
pub fn invitation_into(name: &str) -> String {
    match current() {
        Language::En => format!(
            "It works once and expires in seven days. Whoever takes it chooses their own \
             username, makes a passkey, and administers {name}."
        ),
        Language::Es => format!(
            "Sirve una vez y caduca en siete días. Quien lo tome elige su propio usuario, crea \
             una llave de acceso y administra {name}."
        ),
    }
}

/// A name another organisation already answers on.
pub fn name_is_taken(holder: &str) -> String {
    match current() {
        Language::En => format!("{holder} already answers on that name"),
        Language::Es => format!("{holder} ya responde en ese nombre"),
    }
}

/// A release this node is already running.
pub fn already_running(tag: &str) -> String {
    match current() {
        Language::En => format!("{tag} is what this node is already running"),
        Language::Es => format!("{tag} es lo que este nodo ya está ejecutando"),
    }
}

/// How long ago, from a number of seconds. A negative count is clock skew,
/// or a row written in the same tick.
pub fn ago(seconds: i64) -> String {
    let (count, unit) = match seconds {
        s if s < 60 => (0, Unit::Moment),
        s if s < 3600 => (s / 60, Unit::Minute),
        s if s < 86_400 => (s / 3600, Unit::Hour),
        s => (s / 86_400, Unit::Day),
    };
    match (current(), unit, count) {
        (Language::En, Unit::Moment, _) => "just now".into(),
        (Language::En, Unit::Minute, 1) => "1 minute ago".into(),
        (Language::En, Unit::Minute, n) => format!("{n} minutes ago"),
        (Language::En, Unit::Hour, 1) => "1 hour ago".into(),
        (Language::En, Unit::Hour, n) => format!("{n} hours ago"),
        (Language::En, Unit::Day, 1) => "1 day ago".into(),
        (Language::En, Unit::Day, n) => format!("{n} days ago"),
        (Language::Es, Unit::Moment, _) => "ahora mismo".into(),
        (Language::Es, Unit::Minute, 1) => "hace 1 minuto".into(),
        (Language::Es, Unit::Minute, n) => format!("hace {n} minutos"),
        (Language::Es, Unit::Hour, 1) => "hace 1 hora".into(),
        (Language::Es, Unit::Hour, n) => format!("hace {n} horas"),
        (Language::Es, Unit::Day, 1) => "hace 1 día".into(),
        (Language::Es, Unit::Day, n) => format!("hace {n} días"),
    }
}

enum Unit {
    Moment,
    Minute,
    Hour,
    Day,
}

/// How many of a thing there are, as the heading over their list says it.
pub fn passkeys(count: usize) -> String {
    match (current(), count) {
        (Language::En, 1) => "1 passkey".into(),
        (Language::En, n) => format!("{n} passkeys"),
        (Language::Es, 1) => "1 llave de acceso".into(),
        (Language::Es, n) => format!("{n} llaves de acceso"),
    }
}

pub fn trusted_devices(count: usize) -> String {
    match (current(), count) {
        (Language::En, 1) => "1 trusted device".into(),
        (Language::En, n) => format!("{n} trusted devices"),
        (Language::Es, 1) => "1 dispositivo de confianza".into(),
        (Language::Es, n) => format!("{n} dispositivos de confianza"),
    }
}

/// When a passkey was made and in which browser, which is what tells two
/// from one password manager apart.
pub fn added_in(ago: &str, browser: &str) -> String {
    match current() {
        Language::En => format!("added {ago} in {browser}"),
        Language::Es => format!("creada {ago} en {browser}"),
    }
}

pub fn last_used(ago: &str) -> String {
    match current() {
        Language::En => format!("last used {ago}"),
        Language::Es => format!("último uso {ago}"),
    }
}

pub fn trusted_since(ago: &str) -> String {
    match current() {
        Language::En => format!("trusted {ago}"),
        Language::Es => format!("de confianza {ago}"),
    }
}

/// The passkey a device signed in with, as the request it opened says.
pub fn with_your_passkey(name: &str) -> String {
    match current() {
        Language::En => format!("with your passkey from {name}"),
        Language::Es => format!("con tu llave de {name}"),
    }
}

/// Where the code a trusted device shows has to be typed.
pub fn type_it_in(device: &str) -> String {
    match current() {
        Language::En => format!("Type it in {device}"),
        Language::Es => format!("Escríbelo en {device}"),
    }
}

/// Activity the account has not looked at yet.
pub fn new_in_activity(count: usize) -> String {
    match (current(), count) {
        (Language::En, 1) => "1 new in Activity".into(),
        (Language::En, n) => format!("{n} new in Activity"),
        (Language::Es, 1) => "1 novedad en Actividad".into(),
        (Language::Es, n) => format!("{n} novedades en Actividad"),
    }
}

/// A browser's name, `Safari on macOS`, as it was stored, in the page's
/// language: the browser and the system are names, the word between them
/// is not.
pub fn device(name: &str) -> String {
    match (current(), name.split_once(" on ")) {
        (Language::Es, Some((browser, system))) => format!("{browser} en {system}"),
        (Language::Es, None) if name == "A browser" => "Un navegador".into(),
        _ => name.to_string(),
    }
}

/// When a link or a code stops working, before the island counts it down.
pub fn expires_in(seconds: i64) -> String {
    let minutes = (seconds.max(0) + 59) / 60;
    match (current(), minutes) {
        (Language::En, 1) => "Expires in 1 minute".into(),
        (Language::En, n) => format!("Expires in {n} minutes"),
        (Language::Es, 1) => "Caduca en 1 minuto".into(),
        (Language::Es, n) => format!("Caduca en {n} minutos"),
    }
}

/// Whose account a new device is asking to be let into.
pub fn new_to(username: &str) -> String {
    match current() {
        Language::En => format!("Your passkey worked. This browser is new to {username}."),
        Language::Es => format!("Tu llave funcionó. Este navegador es nuevo para {username}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page in Spanish said "23 minutes ago": the unit was English in a
    /// `format!` that no table could reach.
    #[test]
    fn a_time_ago_is_said_in_the_page_s_language() {
        let english = super::super::language::scoped(Language::En, || ago(23 * 60));
        let spanish = super::super::language::scoped(Language::Es, || ago(23 * 60));
        let one = super::super::language::scoped(Language::Es, || ago(86_400));

        assert_eq!(english, "23 minutes ago");
        assert_eq!(spanish, "hace 23 minutos");
        assert_eq!(one, "hace 1 día");
    }

    /// The number moves. That is the whole reason these are functions:
    /// Spanish puts the count after the verb, and a `format!` at the
    /// call site would have fixed the English order for both.
    #[test]
    fn the_figure_sits_where_each_language_wants_it() {
        let english = super::super::language::scoped(Language::En, || copies_moved(2, &[]));
        let spanish = super::super::language::scoped(Language::Es, || copies_moved(2, &[]));

        assert!(english.starts_with('2'), "{english}");
        assert!(spanish.starts_with("Se movieron"), "{spanish}");
    }
}
