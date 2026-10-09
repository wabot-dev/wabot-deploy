//! What the two people pages say — the machine's own and an
//! organisation's — and the page a stranger opens to accept an
//! invitation.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("Accounts", "Cuentas"),
    ("Administrator — everything", "Administrador — todo"),
    ("Ask whoever invited you for another.", "Pídele otra a quien te invitó."),
    ("At least 12 characters. Nobody here will ever see it.", "Doce caracteres como mínimo. Aquí nadie va a verla nunca."),
    ("Create account", "Crear cuenta"),
    ("Everybody in this organisation. What they can reach is its projects and nothing else on this machine.", "Todo el mundo en esta organización. Lo que alcanzan son sus proyectos y nada más de esta máquina."),
    ("In this organisation", "En esta organización"),
    ("Invitations", "Invitaciones"),
    ("Invitations waiting", "Invitaciones pendientes"),
    ("Join this organisation", "Únete a esta organización"),
    ("Make a member", "Hacer miembro"),
    ("Make an administrator", "Hacer administrador"),
    ("Member — their own projects", "Miembro — sus propios proyectos"),
    ("On this node", "En este nodo"),
    ("This invitation is not valid. It may have been used already, withdrawn, or expired — they last seven days.", "Esta invitación no vale. Puede que ya se usara, que se retirara o que caducara — duran siete días."),
    ("This is you", "Este eres tú"),
    ("You were invited as ", "Te invitaron como "),
];

pub fn t(english: &'static str) -> &'static str {
    crate::web::views::texts::language::translated(english, TABLE)
}

pub fn word(text: &str) -> &str {
    crate::web::views::texts::language::word_in(text, TABLE)
}
