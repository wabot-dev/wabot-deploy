//! The frame around every page: the navigation, the theme and the
//! language toggle.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("A device asking", "Un dispositivo pide entrar"),
    ("A device asks to sign in as you", "Un dispositivo pide entrar como tú"),
    ("Activity", "Actividad"),
    ("Breadcrumb", "Ruta de navegación"),
    ("Devices", "Dispositivos"),
    ("Error: ", "Error: "),
    ("Expires in {}", "Caduca en {}"),
    ("Go to", "Ir a"),
    ("Go to…", "Ir a…"),
    ("It was me", "Fui yo"),
    ("It was not me", "No fui yo"),
    ("Nothing here is called that.", "Nada aquí se llama así."),
    ("Overview", "Resumen"),
    ("Sections", "Secciones"),
    ("Sign out", "Cerrar sesión"),
    ("Status", "Estado"),
    ("This browser cannot use passkeys.", "Este navegador no puede usar llaves de acceso."),
    ("This device already has a passkey for your account.", "Este dispositivo ya tiene una llave de acceso para tu cuenta."),
    ("Type the name of something here to go to it.", "Escribe aquí el nombre de algo para ir a ello."),
    ("Your account", "Tu cuenta"),
    ("Your device gave no passkey. Try again.", "Tu dispositivo no dio ninguna llave de acceso. Inténtalo de nuevo."),
    ("Your device makes a passkey: a fingerprint, a face or a PIN, and nothing to remember.", "Tu dispositivo crea una llave de acceso: una huella, una cara o un PIN, y nada que recordar."),
    ("asks to sign in as you", "pide entrar como tú"),
];
