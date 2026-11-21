//! What a project's pages say: its people, its push tokens, and
//! deleting it.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("A name opens the addresses it resolves to, and with them every other name served from those addresses: a name on a CDN opens the CDN's. Any other name does not resolve, unless the list reaches every address on some port.", "Un nombre abre las direcciones a las que resuelve, y con ellas cualquier otro nombre servido desde esas direcciones: un nombre en una CDN abre las de la CDN. Cualquier otro nombre no resuelve, salvo que la lista llegue a todas las direcciones en algún puerto."),
    ("A node is listed once it lets this one name it as an exit. Private addresses and this project's own network are reached from the node a copy runs on.", "Un nodo aparece aquí cuando permite que este lo nombre como salida. Las direcciones privadas y la red propia de este proyecto se alcanzan desde el nodo donde corre la copia."),
    ("A push and a pull on its own name. The nodes of the network that run its copies are let in to pull whatever it says.", "Un push y un pull en su propio nombre. Los nodos de la red que corren sus copias entran a tirar diga lo que diga."),
    ("Addresses several services let in — an office, a VPN, a CDN's ranges — kept once. A service uses a list on its own Who may connect tab, and lets in its own lines and those of every list it uses.", "Direcciones que dejan entrar varios servicios — una oficina, una VPN, los rangos de una CDN — guardadas una vez. Un servicio usa una lista en su propia pestaña Quién puede conectarse, y deja entrar sus propias líneas y las de cada lista que usa."),
    ("Administered from the node that sent it.", "Se administra desde el nodo que lo envió."),
    ("Asked for by its containers on this node and not on the list, so they were told there is no such name.", "Los pidieron sus contenedores en este nodo y no están en la lista, así que se les dijo que no existen."),
    ("Create group", "Crear grupo"),
    ("Create list", "Crear lista"),
    ("Create project", "Crear proyecto"),
    ("Create token", "Crear token"),
    ("Delete group", "Borrar grupo"),
    ("Delete list", "Borrar lista"),
    ("Delete project", "Borrar proyecto"),
    ("Deleting a project stops and removes every service under it, and takes everything its databases stored on this node. There is no undo.", "Borrar un proyecto para y elimina todos los servicios que tiene dentro, y se lleva todo lo que sus bases de datos guardaron en este nodo. No hay vuelta atrás."),
    ("Destinations", "Destinos"),
    ("Docker keeps one login per name, so projects that share this node's name share one login: signing in with one project's token signs out of the others. A name of this project's own keeps its login beside theirs.", "Docker guarda un inicio de sesión por nombre, así que los proyectos que comparten el nombre de este nodo comparten uno: entrar con el token de un proyecto saca de los demás. Un nombre propio de este proyecto guarda su sesión junto a la de ellos."),
    ("Each copy leaves from the node it runs on.", "Cada copia sale desde el nodo donde corre."),
    ("Internet", "Internet"),
    ("It may connect anywhere.", "Puede conectarse a cualquier lugar."),
    ("Its connections to the internet leave from the chosen nodes' addresses, a different one each time, over TCP: anything else leaving for the internet is dropped. When none of them answers, nothing leaves. None chosen is each copy from its own node.", "Sus conexiones a internet salen desde las direcciones de los nodos elegidos, uno distinto cada vez, por TCP: cualquier otra cosa que salga hacia internet se descarta. Cuando ninguno responde, no sale nada. Sin ninguno elegido, cada copia sale desde su propio nodo."),
    ("Its own name, pointed at this node. Empty is this node's name.", "Su propio nombre, apuntando a este nodo. Vacío es el nombre de este nodo."),
    ("Its own services, its names on other nodes and this node's own addresses are reached whatever the list says. A connection already open stays open until it closes.", "Sus propios servicios, sus nombres en otros nodos y las direcciones de este nodo se alcanzan diga lo que diga la lista. Una conexión ya abierta sigue abierta hasta que se cierre."),
    ("Joined by ", "Lo usan "),
    ("Leaves through", "Sale por"),
    ("Names it was refused", "Nombres que se le negaron"),
    ("New group", "Grupo nuevo"),
    ("New list", "Lista nueva"),
    ("No other node lets this one name it as an exit.", "Ningún otro nodo permite que este lo nombre como salida."),
    ("No service has joined it", "Ningún servicio lo usa"),
    ("No service uses it", "Ningún servicio la usa"),
    ("No services yet.", "Todavía no hay servicios."),
    ("Nobody yet. Administrators of this node reach every project without being added to it.", "Todavía nadie. Los administradores de este nodo llegan a todos los proyectos sin estar añadidos a ellos."),
    ("None. A token is what CI authenticates with — it is nobody's password, and revoking it changes nothing else.", "Ninguno. Un token es con lo que se autentica CI — no es la contraseña de nadie, y revocarlo no cambia nada más."),
    ("Nothing leaves: the list is empty.", "No sale nada: la lista está vacía."),
    ("One address or range per line, and a note after it if you like. Saving changes every service using it at once, with no deployment.", "Una dirección o un rango por línea, y una nota después si quieres. Guardar cambia al instante todos los servicios que la usan, sin despliegue."),
    ("One destination per line: an address, a range, or a name like api.example.com or *.example.com, a port after a colon if it is one port, and a note after it if you like. Empty lets nothing out. Anywhere is 0.0.0.0/0.", "Un destino por línea: una dirección, un rango o un nombre como api.example.com o *.example.com, un puerto tras dos puntos si es un solo puerto, y una nota después si quieres. Vacía no deja salir nada. Cualquier lugar es 0.0.0.0/0."),
    ("Only the destinations below.", "Solo los destinos de abajo."),
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
    ("Where it may connect", "Adónde puede conectarse"),
    ("Who may reach its registry", "Quién puede alcanzar su registry"),
    ("You are: ", "Tu rol: "),
    ("never used", "sin usar"),
    ("no longer carries this node's projects, so nothing leaves through it", "ya no lleva los proyectos de este nodo, así que nada sale por él"),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
