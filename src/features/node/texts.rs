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
    (
        "An instruction was refused",
        "Una instrucción fue rechazada",
    ),
    ("Capabilities", "Capacidades"),
    ("Copies out of the rotation", "Copias fuera de la rotación"),
    ("Empty closes it. A list applies to new connections: a session already open stays open, so closing it from inside one does not end it. If this and the console both shut you out, the machine's own terminal — your provider's serial console — opens it again:", "Vacía lo cierra. Una lista se aplica a las conexiones nuevas: una sesión ya abierta sigue abierta, así que cerrarlo desde dentro de una no la termina. Si esto y la consola te dejan fuera, la terminal de la propia máquina — la consola serie de tu proveedor — lo vuelve a abrir:"),
    ("Every /v2 path on this node's own names: what a push and a pull reach. A node of the network that has called this one is let in to pull the images it runs.", "Cada ruta /v2 en los nombres propios de este nodo: lo que alcanzan un push y un pull. Un nodo de la red que ya llamó a este entra a tirar las imágenes que corre."),
    (
        "Every copy of a database on one machine",
        "Todas las copias de una base en una máquina",
    ),
    ("Its port, as sshd reports it: ", "Su puerto, según sshd: "),
    ("No way to reach a node", "Sin camino hacia un nodo"),
    ("SSH", "SSH"),
    ("Storage nothing claims", "Almacenamiento que nadie reclama"),
    ("The console and its API, on this node's own names. The other nodes of the network reach this one whatever it says. A list that does not hold the address you are saving it from is refused: it would lock you out.", "La consola y su API, en los nombres propios de este nodo. Los otros nodos de la red alcanzan a este diga lo que diga. Una lista que no contiene la dirección desde la que la guardas se rechaza: te dejaría fuera."),
    ("The disk is nearly full", "El disco está casi lleno"),
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
