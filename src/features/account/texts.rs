//! Your account: passkeys, devices and what happened.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (". Somebody else has it.", ". Alguien más la tiene."),
    ("Add a device", "Añadir un dispositivo"),
    ("Add a passkey", "Añadir una llave de acceso"),
    ("Added", "Añadida"),
    ("Device", "Dispositivo"),
    ("Device trusted", "Dispositivo de confianza"),
    ("Device withdrawn", "Dispositivo retirado"),
    ("Its passkey passed. If it was you, it shows six digits to type there.", "Su llave de acceso pasó. Si fuiste tú, muestra seis dígitos para escribir allí."),
    ("Last used", "Último uso"),
    ("Made on", "Creada en"),
    ("No device is asking.", "Ningún dispositivo pide entrar."),
    ("No device is trusted. Signing in will need a link.", "Ningún dispositivo es de confianza. Para entrar hará falta un enlace."),
    ("No passkeys yet.", "Aún no hay llaves de acceso."),
    ("Nothing has happened yet.", "Aún no ha pasado nada."),
    ("Only a device you trust can let another in, and this one is not trusted.", "Solo un dispositivo de confianza puede dejar entrar a otro, y este no lo es."),
    ("Open this on the new device within five minutes. It makes its own passkey there, and is trusted.", "Ábrelo en el dispositivo nuevo antes de cinco minutos. Allí crea su propia llave de acceso y queda como de confianza."),
    ("Passkey added", "Llave de acceso añadida"),
    ("Passkey removed", "Llave de acceso eliminada"),
    ("Passkeys", "Llaves de acceso"),
    ("Passkeys need this console opened by its name.", "Las llaves de acceso necesitan esta consola abierta por su nombre."),
    ("Password deleted", "Contraseña eliminada"),
    ("Remove this passkey", "Eliminar esta llave de acceso"),
    ("Sign-in approved", "Entrada aprobada"),
    ("Sign-in link used", "Enlace de acceso usado"),
    ("Sign-in refused", "Entrada rechazada"),
    ("Signed out everywhere", "Sesión cerrada en todas partes"),
    ("That was shown once, and is gone.", "Eso se mostró una vez, y ya no está."),
    ("The device you refused signed in with ", "El dispositivo que rechazaste entró con "),
    ("Type this on the other device", "Escribe esto en el otro dispositivo"),
    ("You still have a password. Your first passkey deletes it.", "Aún tienes una contraseña. Tu primera llave de acceso la elimina."),
    ("for", "para"),
    ("synced", "sincronizada"),
    ("this device", "este dispositivo"),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
