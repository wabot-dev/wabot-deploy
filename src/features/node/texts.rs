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
        "A database is not archiving its log",
        "Una base no está archivando su log",
    ),
    (
        "A node this one cannot reach",
        "Un nodo que este no alcanza",
    ),
    ("A push holds its own bytes for a day, so an image pushed less than a day ago gives its disk back within the hour after its push is a day old.", "Un push retiene sus propios bytes durante un día, así que una imagen subida hace menos de un día devuelve su disco dentro de la hora siguiente a cumplir un día."),
    ("A store runs Garage, an engine published under the GNU AGPLv3, in a container of its own. Turning this on lets projects here make stores; turning it off stops none that exist, and refuses to start or make one.", "Un almacén corre Garage, un motor publicado bajo la GNU AGPLv3, en un contenedor propio. Activarlo deja que los proyectos de aquí creen almacenes; desactivarlo no detiene ninguno de los que existen, y se niega a arrancar o crear uno."),
    (
        "A tunnel that stopped working",
        "Un túnel que dejó de funcionar",
    ),
    (
        "An instruction was refused",
        "Una instrucción fue rechazada",
    ),
    ("Capabilities", "Capacidades"),
    ("Copies out of the rotation", "Copias fuera de la rotación"),
    ("DNS server", "Servidor DNS"),
    ("Empty closes it. A list applies to new connections: a session already open stays open, so closing it from inside one does not end it. If this and the console both shut you out, the machine's own terminal — your provider's serial console — opens it again:", "Vacía lo cierra. Una lista se aplica a las conexiones nuevas: una sesión ya abierta sigue abierta, así que cerrarlo desde dentro de una no la termina. Si esto y la consola te dejan fuera, la terminal de la propia máquina — la consola serie de tu proveedor — lo vuelve a abrir:"),
    ("Every /v2 path on this node's own names: what a push and a pull reach. A node of the network that has called this one is let in to pull the images it runs.", "Cada ruta /v2 en los nombres propios de este nodo: lo que alcanzan un push y un pull. Un nodo de la red que ya llamó a este entra a tirar las imágenes que corre."),
    (
        "Every copy of a database on one machine",
        "Todas las copias de una base en una máquina",
    ),
    ("Images kept", "Imágenes que se guardan"),
    ("Its port, as sshd reports it: ", "Su puerto, según sshd: "),
    ("No way to reach a node", "Sin camino hacia un nodo"),
    ("Nothing does.", "Nada."),
    ("Per service", "Por servicio"),
    ("Run stores of buckets, which speak S3", "Correr almacenes de buckets, que hablan S3"),
    ("SSH", "SSH"),
    ("Storage nothing claims", "Almacenamiento que nadie reclama"),
    ("The console and its API, on this node's own names. The other nodes of the network reach this one whatever it says. A list that does not hold the address you are saving it from is refused: it would lock you out.", "La consola y su API, en los nombres propios de este nodo. Los otros nodos de la red alcanzan a este diga lo que diga. Una lista que no contiene la dirección desde la que la guardas se rechaza: te dejaría fuera."),
    ("The disk is nearly full", "El disco está casi lleno"),
    ("The image a service runs and the newest one stay whatever the number. The rest go, oldest first, and their versions and tags with them. Empty means the default of two.", "La imagen que corre un servicio y la más nueva se quedan diga lo que diga el número. Las demás se van, de la más vieja a la más nueva, y con ellas sus versiones y sus tags. Vacío es el valor por defecto, dos."),
    ("The registry under this node's name", "El registry bajo el nombre de este nodo"),
    ("This console", "Esta consola"),
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
