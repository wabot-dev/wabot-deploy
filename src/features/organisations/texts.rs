//! What the organisations pages say.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (" person/people", " persona(s)"),
    (" project(s)", " proyecto(s)"),
    ("A link that works once and expires in seven days. Whoever takes it chooses their own username and password, and administers this organisation.", "Un enlace que sirve una vez y caduca en siete días. Quien lo tome elige su propio usuario y contraseña, y administra esta organización."),
    ("An organisation has its own projects, its own people and its own console. What stays this node's is the machine underneath it.", "Una organización tiene sus propios proyectos, su propia gente y su propia consola. Lo que sigue siendo de este nodo es la máquina de debajo."),
    ("Answers for it", "Responde por ella"),
    ("Console", "Consola"),
    ("Delete organisation and everything in it", "Eliminar la organización y todo lo que contiene"),
    ("Deleting an organisation stops and removes every service it runs, on this node and on any other, and takes its projects and its people with it. There is no undo.", "Eliminar una organización para y borra cada servicio que corre, en este nodo y en cualquier otro, y se lleva por delante sus proyectos y su gente. No hay vuelta atrás."),
    ("Empty is no ceiling. Memory like 512 MB or 2 GB; CPU in cores like 0.5 or 2, or millicores with an m, like 500m.", "Vacío es sin techo. Memoria como 512 MB o 2 GB; CPU en núcleos como 0.5 o 2, o en millicores con una m, como 500m."),
    ("Empty takes the console away. Its projects and its people stay; there is nowhere to administer them from.", "Vacío le quita la consola. Sus proyectos y su gente siguen ahí; no queda desde dónde administrarlos."),
    ("Invite an administrator", "Invitar a un administrador"),
    ("It has no domain yet, so the link will be on this node's name — the account is made, and the console it belongs to exists once you give the organisation one.", "Todavía no tiene dominio, así que el enlace irá con el nombre de este nodo — la cuenta se crea, y la consola a la que pertenece existe en cuanto le des uno a la organización."),
    ("It has no domain, so it has no console yet.", "No tiene dominio, así que todavía no tiene consola."),
    ("It is summed over every copy of every service this organisation runs, wherever it runs — not what this machine holds. What a service may take is what counts against it: the ceiling is the reservation.", "Se suma sobre cada copia de cada servicio que corre esta organización, dondequiera que corra — no sobre lo que sostiene esta máquina. Lo que un servicio puede tomar es lo que cuenta contra ella: el techo es la reserva."),
    ("Its administrators. Who else is in it, and what they reach, is theirs to decide from their own console.", "Sus administradores. Quién más está dentro, y qué alcanza, lo deciden ellos desde su propia consola."),
    ("Its projects and its people are its own: an administrator of one reaches nothing outside it, and this node's operator still sees everything the machine is running. Give it a domain to bring up its console.", "Sus proyectos y su gente son suyos: un administrador de una no alcanza nada fuera de ella, y el operador de este nodo sigue viendo todo lo que corre la máquina. Dale un dominio para levantar su consola."),
    ("Its projects and its people exist; there is nowhere to administer them from. Give it one in Settings.", "Sus proyectos y su gente existen; no hay desde dónde administrarlos. Dale uno en Ajustes."),
    ("Listed, not managed. A project is administered from the console it was made in.", "Listados, no administrados. Un proyecto se administra desde la consola en la que se creó."),
    ("New organisation", "Nueva organización"),
    ("No domain", "Sin dominio"),
    ("Nobody answers for this organisation. Until somebody does, nothing inside it can be changed at all.", "Nadie responde por esta organización. Hasta que alguien lo haga, no se puede cambiar nada dentro de ella."),
    ("Nobody. Invite an administrator in Settings — until then nothing inside this organisation can be changed by anybody.", "Nadie. Invita a un administrador en Ajustes — hasta entonces nadie puede cambiar nada dentro de esta organización."),
    ("None yet. Its administrator makes the first one.", "Todavía ninguno. Su administrador crea el primero."),
    ("Owner", "Dueño"),
    ("Quota", "Cuota"),
    ("Slug", "Slug"),
    ("Summed over every copy of every service, on this node and on any other that runs one. What a service may take is what counts — the ceiling is the reservation.", "Sumado sobre cada copia de cada servicio, en este nodo y en cualquier otro que corra uno. Lo que un servicio puede tomar es lo que cuenta — el techo es la reserva."),
    ("The only people inside an organisation this node's operator decides about. Who else is in it, and what they reach, is theirs — from their own console.", "La única gente dentro de una organización sobre la que decide el operador de este nodo. Quién más está dentro, y qué alcanza, es cosa suya — desde su propia consola."),
    ("This node is not shared out to anybody yet.", "Este nodo todavía no se reparte con nadie."),
    ("What it has taken", "Lo que ha tomado"),
    ("Where its console answers, and the suffix its service names hang off. It has to point at this node, and it cannot be one of this node's own names — the console an operator fixes things from must not be reachable only through somebody else's.", "Dónde responde su consola, y el sufijo del que cuelgan los nombres de sus servicios. Tiene que apuntar a este nodo, y no puede ser uno de los nombres propios de este nodo — la consola desde la que un operador arregla las cosas no puede alcanzarse solo a través de la de otro."),
    ("Where its people sign in, and the suffix its service names hang off.", "Dónde entra su gente, y el sufijo del que cuelgan los nombres de sus servicios."),
    ("Who this node is shared out to.", "Entre quiénes se reparte este nodo."),
];
