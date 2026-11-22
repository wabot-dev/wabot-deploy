//! Signing in, and claiming a node that has nobody on it yet.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("Add this device", "Añadir este dispositivo"),
    ("Code", "Código"),
    ("Confirm it from another device", "Confírmalo desde otro dispositivo"),
    ("Create administrator", "Crear administrador"),
    ("Lost your device? Whoever runs this console can give you a sign-in link.", "¿Perdiste tu dispositivo? Quien administra esta consola puede darte un enlace de acceso."),
    ("No setup token is outstanding, so nobody can be created from here. Issue one on the node:", "No hay ningún token de instalación pendiente, así que no se puede crear a nadie desde aquí. Emite uno en el nodo:"),
    ("Not on a computer somebody else uses.", "No en un equipo que use otra persona."),
    ("Open this console on a device you trust.", "Abre esta consola en un dispositivo de confianza."),
    ("Opened by its address, this node cannot make passkeys. Sign in again with a link from `wabot-deploy sign-in-link`.", "Abierto por su dirección, este nodo no puede crear llaves de acceso. Vuelve a entrar con un enlace de `wabot-deploy sign-in-link`."),
    ("Passkeys work only with this console's name.", "Las llaves de acceso solo funcionan con el nombre de esta consola."),
    ("Press the account icon there, then It was me.", "Pulsa allí el icono de la cuenta y luego Fui yo."),
    ("Set up this node", "Configurar este nodo"),
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
    ("Type the six digits it shows.", "Escribe los seis dígitos que muestre."),
    ("Without it, sign in with a link from the node:", "Sin él, entra con un enlace del nodo:"),
    ("Your device makes a passkey for this console as you sign in.", "Tu dispositivo crea una llave de acceso para esta consola al entrar."),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
