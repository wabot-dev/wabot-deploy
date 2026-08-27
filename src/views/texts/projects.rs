//! What a project's pages say: its people, its push tokens, and
//! deleting it.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("Create project", "Crear proyecto"),
    ("Create token", "Crear token"),
    ("Delete project", "Borrar proyecto"),
    ("Deleting a project stops and removes every service under it, and takes everything its databases stored on this node. There is no undo.", "Borrar un proyecto para y elimina todos los servicios que tiene dentro, y se lleva todo lo que sus bases de datos guardaron en este nodo. No hay vuelta atrás."),
    ("Evicted there", "Expulsada allí"),
    ("No services yet.", "Todavía no hay servicios."),
    ("Nobody yet. Administrators of this node reach every project without being added to it.", "Todavía nadie. Los administradores de este nodo llegan a todos los proyectos sin estar añadidos a ellos."),
    ("None. A token is what CI authenticates with — it is nobody's password, and revoking it changes nothing else.", "Ninguno. Un token es con lo que se autentica CI — no es la contraseña de nadie, y revocarlo no cambia nada más."),
    ("Not answering", "No responde"),
    ("Person", "Persona"),
    ("Push tokens", "Tokens de subida"),
    ("Running elsewhere", "Corriendo en otro nodo"),
    ("Shown once. Use it as the password: ", "Se muestra una vez. Úsalo como contraseña: "),
    ("Somebody who already has an account on this node — adding them here does not create one. A new person is invited from Settings, People.", "Alguien que ya tiene cuenta en este nodo — añadirla aquí no crea ninguna. A una persona nueva se la invita desde Ajustes, Personas."),
    ("The slug is derived from the name, and it is what hostnames and containerd labels are built from.", "El slug se deriva del nombre, y es con lo que se construyen los nombres de host y las etiquetas de containerd."),
    ("This node owns nothing, by its own setting. It runs what other nodes ask it to run; anything that arrives that way is administered from the node that sent it.", "Este nodo no es dueño de nada, por decisión propia. Corre lo que otros nodos le piden correr; lo que llega así se administra desde el nodo que lo envió."),
    ("You are: ", "Tu rol: "),
    ("never used", "sin usar"),
];
