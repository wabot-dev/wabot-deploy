//! Signing in, and claiming a node that has nobody on it yet.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("Add this device", "Añadir este dispositivo"),
    ("Code", "Código"),
    ("Create administrator", "Crear administrador"),
    ("No setup token is outstanding, so nobody can be created from here. Issue one on the node:", "No hay ningún token de instalación pendiente, así que no se puede crear a nadie desde aquí. Emite uno en el nodo:"),
    ("Open this console on a device you trust and choose It was me. Type the six digits it shows.", "Abre esta consola en un dispositivo de confianza y elige Fui yo. Escribe los seis dígitos que muestre."),
    ("Opened by its address, this node cannot make passkeys. Sign in again with a link from `wabot-deploy sign-in-link`.", "Abierto por su dirección, este nodo no puede crear llaves de acceso. Vuelve a entrar con un enlace de `wabot-deploy sign-in-link`."),
    ("Passkeys need this console opened by its name. Without one, ask for a sign-in link.", "Las llaves de acceso necesitan esta consola abierta por su nombre. Sin él, pide un enlace de acceso."),
    ("Setup token", "Token de instalación"),
    ("Sign in", "Entrar"),
    ("Sign in again", "Vuelve a entrar"),
    ("Sign in with a passkey", "Entrar con una llave de acceso"),
    ("Sign in with a password", "Entrar con una contraseña"),
    ("Signing in as ", "Entrando como "),
    ("That request is closed.", "Esa solicitud está cerrada."),
    ("The setup token was printed by `wabot-deploy install`. It works once, and it expires.", "El token de instalación lo imprimió `wabot-deploy install`. Funciona una vez, y caduca."),
    ("This link is not valid. It may have been used already, or expired.", "Este enlace no es válido. Puede que ya se haya usado, o que haya caducado."),
    ("Trust this device", "Confiar en este dispositivo"),
    ("Your device makes a passkey for this console as you sign in.", "Tu dispositivo crea una llave de acceso para esta consola al entrar."),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
