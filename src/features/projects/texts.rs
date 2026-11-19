//! What a project's pages say: its people, its push tokens, and
//! deleting it.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("A push and a pull on its own name. The nodes of the network that run its copies are let in to pull whatever it says.", "Un push y un pull en su propio nombre. Los nodos de la red que corren sus copias entran a tirar diga lo que diga."),
    ("Addresses several services let in — an office, a VPN, a CDN's ranges — kept once. A service uses a list on its own Who may connect tab, and lets in its own lines and those of every list it uses.", "Direcciones que dejan entrar varios servicios — una oficina, una VPN, los rangos de una CDN — guardadas una vez. Un servicio usa una lista en su propia pestaña Quién puede conectarse, y deja entrar sus propias líneas y las de cada lista que usa."),
    ("Create group", "Crear grupo"),
    ("Create list", "Crear lista"),
    ("Create project", "Crear proyecto"),
    ("Create token", "Crear token"),
    ("Delete group", "Borrar grupo"),
    ("Delete list", "Borrar lista"),
    ("Delete project", "Borrar proyecto"),
    ("Deleting a project stops and removes every service under it, and takes everything its databases stored on this node. There is no undo.", "Borrar un proyecto para y elimina todos los servicios que tiene dentro, y se lleva todo lo que sus bases de datos guardaron en este nodo. No hay vuelta atrás."),
    ("Docker keeps one login per name, so projects that share this node's name share one login: signing in with one project's token signs out of the others. A name of this project's own keeps its login beside theirs.", "Docker guarda un inicio de sesión por nombre, así que los proyectos que comparten el nombre de este nodo comparten uno: entrar con el token de un proyecto saca de los demás. Un nombre propio de este proyecto guarda su sesión junto a la de ellos."),
    ("Its own name, pointed at this node. Empty is this node's name.", "Su propio nombre, apuntando a este nodo. Vacío es el nombre de este nodo."),
    ("Joined by ", "Lo usan "),
    ("New group", "Grupo nuevo"),
    ("New list", "Lista nueva"),
    ("No service has joined it", "Ningún servicio lo usa"),
    ("No service uses it", "Ningún servicio la usa"),
    ("No services yet.", "Todavía no hay servicios."),
    ("Nobody yet. Administrators of this node reach every project without being added to it.", "Todavía nadie. Los administradores de este nodo llegan a todos los proyectos sin estar añadidos a ellos."),
    ("None. A token is what CI authenticates with — it is nobody's password, and revoking it changes nothing else.", "Ninguno. Un token es con lo que se autentica CI — no es la contraseña de nadie, y revocarlo no cambia nada más."),
    ("One address or range per line, and a note after it if you like. Saving changes every service using it at once, with no deployment.", "Una dirección o un rango por línea, y una nota después si quieres. Guardar cambia al instante todos los servicios que la usan, sin despliegue."),
    ("Person", "Persona"),
    ("Push tokens", "Tokens de subida"),
    ("Registry", "Registro"),
    ("Saving redeploys every running service that joined it, with the release it is on.", "Guardar redespliega cada servicio en marcha que lo usa, con la release en la que está."),
    ("Shared allowlists", "Listas de acceso compartidas"),
    ("Shared environment", "Entorno compartido"),
    ("Shown once. Use it as the password: ", "Se muestra una vez. Úsalo como contraseña: "),
    ("Somebody who already has an account on this node — adding them here does not create one. A new person is invited from Settings, People.", "Alguien que ya tiene cuenta en este nodo — añadirla aquí no crea ninguna. A una persona nueva se la invita desde Ajustes, Personas."),
    ("The slug is derived from the name, and it is what hostnames and containerd labels are built from.", "El slug se deriva del nombre, y es con lo que se construyen los nombres de host y las etiquetas de containerd."),
    ("This node owns nothing, by its own setting. It runs what other nodes ask it to run; anything that arrives that way is administered from the node that sent it.", "Este nodo no es dueño de nada, por decisión propia. Corre lo que otros nodos le piden correr; lo que llega así se administra desde el nodo que lo envió."),
    ("Used by ", "La usan "),
    ("Variables", "Variables"),
    ("Variables several services read — an SMTP account, an error tracker's key — kept once. A service joins a group on its own Environment tab, and a variable of its own wins over the same one in a group.", "Variables que leen varios servicios — una cuenta SMTP, la clave de un rastreador de errores — guardadas una vez. Un servicio se une a un grupo en su propia pestaña Entorno, y una variable propia gana sobre la misma en un grupo."),
    ("Who may reach its registry", "Quién puede alcanzar su registry"),
    ("You are: ", "Tu rol: "),
    ("never used", "sin usar"),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
