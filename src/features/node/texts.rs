//! What this machine's own settings say — its domain, its reserves,
//! what it will do for anybody — and what the node says needs somebody.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("A backup did not work", "Un backup no funcionó"),
    (
        "A certificate would not issue",
        "Un certificado no se pudo emitir",
    ),
    ("A copy will not start", "Una copia no arranca"),
    (
        "A node this one cannot reach",
        "Un nodo que este no alcanza",
    ),
    (
        "An instruction was refused",
        "Una instrucción fue rechazada",
    ),
    ("Capabilities", "Capacidades"),
    ("Copies out of the rotation", "Copias fuera de la rotación"),
    (
        "Every copy of a database on one machine",
        "Todas las copias de una base en una máquina",
    ),
    ("No way to reach a node", "Sin camino hacia un nodo"),
    ("Storage nothing claims", "Almacenamiento que nadie reclama"),
    (
        "Two nodes claim one address",
        "Dos nodos reclaman una dirección",
    ),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}

pub fn word(text: &str) -> &str {
    crate::shell::texts::language::word_in(text, TABLE)
}
