//! What the organisations page says.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (" project(s)", " proyecto(s)"),
    ("Delete organisation", "Eliminar organización"),
    ("Empty is no ceiling. Memory like 512 MB or 2 GB; CPU in cores like 0.5 or 2, or millicores with an m, like 500m. What a service may take is what counts against this, summed over every copy wherever it runs — the limit is the reservation, here as everywhere else.", "Vacío es sin techo. Memoria como 512 MB o 2 GB; CPU en núcleos como 0.5 o 2, o en millicores con una m, como 500m. Lo que un servicio puede tomar es lo que cuenta contra esto, sumado sobre cada copia dondequiera que corra — el límite es la reserva, aquí como en todo lo demás."),
    ("In it", "En ella"),
    ("Invite somebody to this organisation", "Invitar a alguien a esta organización"),
    ("It has no domain yet, so the link will be on this node's name — the account is made, and the console it belongs to exists once you give the organisation one.", "Todavía no tiene dominio, así que el enlace irá con el nombre de este nodo — la cuenta se crea, y la consola a la que pertenece existe en cuanto le des uno a la organización."),
    ("It has no projects yet. Whoever you invite as an administrator can make the first one.", "Todavía no tiene proyectos. Quien invites como administrador puede crear el primero."),
    ("Its projects and its people are its own: an administrator of one reaches nothing outside it, and this node's operator still sees everything the machine is running. Give it a domain to bring up its console.", "Sus proyectos y su gente son suyos: un administrador de una no alcanza nada fuera de ella, y el operador de este nodo sigue viendo todo lo que corre la máquina. Dale un dominio para levantar su consola."),
    ("New organisation", "Nueva organización"),
    ("Save quota", "Guardar cuota"),
    ("Where its console answers, and the suffix its service names hang off. It has to point at this node, and it cannot be one of this node's own names — the console an operator fixes things from must not be reachable only through somebody else's.", "Dónde responde su consola, y el sufijo del que cuelgan los nombres de sus servicios. Tiene que apuntar a este nodo, y no puede ser uno de los nombres propios de este nodo — la consola desde la que un operador arregla las cosas no puede alcanzarse solo a través de la de otro."),
    ("Who this node is shared out to", "Entre quiénes se reparte este nodo"),
];
