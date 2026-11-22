//! The frame around every page: the navigation, the theme and the
//! language toggle.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("A device asking", "Un dispositivo pide entrar"),
    ("Activity", "Actividad"),
    ("Breadcrumb", "Ruta de navegación"),
    ("Devices", "Dispositivos"),
    ("Error: ", "Error: "),
    ("Go to", "Ir a"),
    ("Go to…", "Ir a…"),
    ("It was me", "Fui yo"),
    ("It was not me", "No fui yo"),
    ("Nothing here is called that.", "Nada aquí se llama así."),
    ("Overview", "Resumen"),
    ("Sections", "Secciones"),
    ("Sign out", "Cerrar sesión"),
    ("Something happened to your account since you last looked:", "Algo pasó en tu cuenta desde la última vez que miraste:"),
    ("Status", "Estado"),
    ("This browser cannot use passkeys.", "Este navegador no puede usar llaves de acceso."),
    ("Type the name of something here to go to it.", "Escribe aquí el nombre de algo para ir a ello."),
    ("Your account", "Tu cuenta"),
    ("Your device gave no passkey. Try again.", "Tu dispositivo no dio ninguna llave de acceso. Inténtalo de nuevo."),
    ("Your device makes a passkey: a fingerprint, a face or a PIN, and nothing to remember.", "Tu dispositivo crea una llave de acceso: una huella, una cara o un PIN, y nada que recordar."),
    ("asks to sign in as you.", "pide entrar como tú."),
    ("in Activity", "en Actividad"),
];
