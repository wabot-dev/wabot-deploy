//! What the updates pages say.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (" is available.", " está disponible."),
    (" · pre-release", " · versión previa"),
    (" · published ", " · publicada "),
    ("Check again", "Comprobar de nuevo"),
    ("Could not read the release list: ", "No se pudo leer la lista de versiones: "),
    ("Earlier attempts", "Intentos anteriores"),
    ("Finished", "Terminada"),
    ("Install", "Instalar"),
    ("Install this release", "Instalar esta versión"),
    ("Installed", "Instalada"),
    ("Installing", "Instalando"),
    ("Last update", "Última actualización"),
    ("No build here", "Sin binario aquí"),
    ("Read it on GitHub", "Leerlo en GitHub"),
    ("The node restarts on its own when the new binary is in place. This page follows along — there is nothing to reload.", "El nodo se reinicia solo cuando el binario nuevo está en su sitio. Esta página lo sigue — no hay nada que recargar."),
    ("This is the newest release published.", "Esta es la versión más nueva publicada."),
    ("This node installs a release when you ask it to, and never on its own. Installing one restarts the node; the containers on it keep running.", "Este nodo instala una versión cuando se lo pides, y nunca por su cuenta. Instalar una reinicia el nodo; los contenedores que tiene siguen corriendo."),
    ("This release came with no notes.", "Esta versión vino sin notas."),
    ("What is in it", "Qué lleva"),
    ("newer than what is running", "más nueva que la que corre"),
    ("no build for this machine", "no hay binario para esta máquina"),
    ("older than what is running", "más vieja que la que corre"),
];
