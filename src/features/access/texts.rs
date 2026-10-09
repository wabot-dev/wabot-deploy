//! Signing in, and claiming a node that has nobody on it yet.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("At least 12 characters. A phrase beats a puzzle — this console can start containers on the machine.", "Doce caracteres como mínimo. Una frase vale más que un acertijo — esta consola puede arrancar contenedores en la máquina."),
    ("Create administrator", "Crear administrador"),
    ("No setup token is outstanding, so nobody can be created from here. Issue one on the node:", "No hay ningún token de instalación pendiente, así que no se puede crear a nadie desde aquí. Emite uno en el nodo:"),
    ("Setup token", "Token de instalación"),
    ("Sign in", "Entrar"),
    ("The setup token was printed by `wabot-deploy install`. It works once, and it expires.", "El token de instalación lo imprimió `wabot-deploy install`. Funciona una vez, y caduca."),
];

pub fn t(english: &'static str) -> &'static str {
    crate::web::views::texts::language::translated(english, TABLE)
}
