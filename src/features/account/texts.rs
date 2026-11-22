//! Your account: passkeys, devices and what happened.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (". Somebody else has it.", ". Alguien más la tiene."),
    ("Add a device", "Añadir un dispositivo"),
    ("Add a passkey", "Añadir una llave de acceso"),
    ("Device trusted", "Dispositivo de confianza"),
    ("Device withdrawn", "Dispositivo retirado"),
    (
        "Empty goes back to where it lives.",
        "Vacío vuelve al nombre de donde vive.",
    ),
    (
        "It is your only passkey. Without it, signing in needs a sign-in link.",
        "Es tu única llave de acceso. Sin ella, para entrar hará falta un enlace de acceso.",
    ),
    (
        "It makes its own passkey there, and is trusted.",
        "Allí crea su propia llave de acceso y queda como de confianza.",
    ),
    ("Make a passkey", "Crear una llave de acceso"),
    ("No device is asking.", "Ningún dispositivo pide entrar."),
    (
        "No device is trusted. Signing in will need a link.",
        "Ningún dispositivo es de confianza. Para entrar hará falta un enlace.",
    ),
    ("Not synced", "Sin sincronizar"),
    ("Nothing has happened yet.", "Aún no ha pasado nada."),
    (
        "Only a device you trust can let another in, and this one is not trusted.",
        "Solo un dispositivo de confianza puede dejar entrar a otro, y este no lo es.",
    ),
    (
        "Open it on the new device",
        "Ábrelo en el dispositivo nuevo",
    ),
    ("Passkey added", "Llave de acceso añadida"),
    ("Passkey removed", "Llave de acceso quitada"),
    ("Passkeys", "Llaves de acceso"),
    (
        "Passkeys need this console opened by its name.",
        "Las llaves de acceso necesitan esta consola abierta por su nombre.",
    ),
    ("Password deleted", "Contraseña eliminada"),
    ("Remove anyway", "Quitar igualmente"),
    ("Remove passkey", "Quitar la llave de acceso"),
    ("Remove this passkey", "Quitar esta llave de acceso"),
    ("Rename", "Renombrar"),
    ("Security key", "Llave física"),
    ("Sign in without a password", "Entra sin contraseña"),
    ("Sign-in approved", "Entrada aprobada"),
    ("Sign-in link used", "Enlace de acceso usado"),
    ("Sign-in refused", "Entrada rechazada"),
    ("Signed out everywhere", "Sesión cerrada en todas partes"),
    ("Stop trusting", "Dejar de confiar"),
    ("Synced", "Sincronizada"),
    (
        "That was shown once, and is gone.",
        "Eso se mostró una vez, y ya no está.",
    ),
    (
        "The device you refused signed in with ",
        "El dispositivo que rechazaste entró con ",
    ),
    (
        "Your device makes a passkey with your fingerprint, face or screen lock.",
        "Tu dispositivo crea una llave de acceso con tu huella, tu cara o el bloqueo de pantalla.",
    ),
    (
        "Your first passkey deletes your password.",
        "Tu primera llave de acceso borra tu contraseña.",
    ),
    ("for", "para"),
    ("not used yet", "aún sin usar"),
    ("this device", "este dispositivo"),
    ("works once", "sirve una vez"),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
