//! What the network's pages say: the nodes this one knows, enrolling
//! and joining, and what one node has asked of another.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (" copies here have no memory ceiling, so what is promised is a floor rather than a total — any of them may take what is left.", " copias de aquí no tienen techo de memoria, así que lo prometido es un mínimo y no un total — cualquiera de ellas puede llevarse lo que queda."),
    (" · this node", " · este nodo"),
    ("A key and an address appear the first time this node enrols another one or joins one itself. The overlay is what carries traffic between nodes — an edge here reaching a container that runs somewhere else.", "La clave y la dirección aparecen la primera vez que este nodo enrola a otro o se une a uno. La overlay es lo que lleva el tráfico entre nodos — un edge de aquí llegando a un contenedor que corre en otro sitio."),
    ("A note, if you want one", "Una nota, si quieres"),
    ("Allowed", "Permitido"),
    ("And offer it, in return:", "Y ofrecerle, a cambio:"),
    ("As reported ", "Según reportó "),
    ("Ask that node to let this one:", "Pedirle a ese nodo que deje a este:"),
    ("Before you join", "Antes de unirte"),
    ("Both lists travel inside the token and are shown on the other machine before it is spent. Whoever holds it accepts or refuses each one — so what is asked for here is a request, not a setting.", "Las dos listas viajan dentro del token y se muestran en la otra máquina antes de gastarlo. Quien lo tenga acepta o rechaza cada una — así que lo que se pide aquí es una petición, no un ajuste."),
    ("Collected", "Recogido"),
    ("Create join token", "Crear token de unión"),
    ("Draining since ", "Drenando desde "),
    ("Forget", "Olvidar"),
    ("Forget this node", "Olvidar este nodo"),
    ("From the network page of the node you are joining: invite a node there, and it shows one token, once. Pasting it here does what `wabot-deploy join` does in a terminal — this node records that one as an authority and tells it so. The same token can be pasted again if something goes wrong part-way.", "Desde la página de red del nodo al que te unes: invita un nodo allí, y enseña un token, una vez. Pegarlo aquí hace lo que hace `wabot-deploy join` en una terminal — este nodo anota aquel como autoridad y se lo dice. El mismo token se puede volver a pegar si algo sale mal a medias."),
    ("How full it is", "Qué tan lleno está"),
    ("Id", "Id"),
    ("Join token", "Token de unión"),
    ("Joined", "Unido"),
    ("May", "Puede"),
    ("Moves the copies this node put there to somewhere with room, and stops choosing it for new ones. Copies that arrived from another node are not this one's to move; a database copy that accepts writes is named rather than moved, because promoting is a decision somebody makes.", "Mueve las copias que este nodo puso allí a algún sitio con espacio, y deja de elegirlo para las nuevas. Las copias que llegaron de otro nodo no son de este para moverlas; una copia de base de datos que acepta escrituras se nombra en vez de moverse, porque promover es una decisión que toma alguien."),
    ("Nodes this one has granted authority to. Revoking is one row, and it takes effect here — a node that has been revoked can ask for nothing.", "Nodos a los que este ha concedido autoridad. Revocar es una fila, y surte efecto aquí — un nodo revocado no puede pedir nada."),
    ("Nothing yet. A node reports this every fifteen seconds once it is running a version that sends it — until then this says nothing rather than nought, which would read as an idle machine.", "Nada todavía. Un nodo reporta esto cada quince segundos en cuanto corre una versión que lo envía — hasta entonces esto no dice nada, en vez de cero, que se leería como una máquina en reposo."),
    ("Nothing. It is handing this node an address on its overlay and asking for nothing back.", "Nada. Le está dando a este nodo una dirección en su overlay y no pide nada a cambio."),
    ("Nothing. This node will take its instructions and give it none.", "Nada. Este nodo tomará sus instrucciones y no le dará ninguna."),
    ("On this network", "En esta red"),
    ("One of these directories was too big to walk while a page was waiting, so its figure is a floor and \"everything else\" is the larger for it.", "Uno de estos directorios era demasiado grande para recorrerlo mientras una página esperaba, así que su cifra es un mínimo y «todo lo demás» es mayor por ello."),
    ("One per running container. The runtime's overhead, not the image's.", "Uno por contenedor en marcha. El coste del runtime, no el de la imagen."),
    ("Only to tell one unspent token from another. The node names itself when it joins, and that \
         is the name this console shows.", "Sólo para distinguir un token sin gastar de otro. El nodo se nombra a sí mismo al unirse, \
         y ese es el nombre que muestra esta consola."),
    ("Only what it was granted, which is listed beside it below. Revoking any of it takes effect here and immediately.", "Solo lo que se le concedió, que está listado a su lado abajo. Revocar cualquier parte surte efecto aquí y de inmediato."),
    ("Overlay address", "Dirección overlay"),
    ("Promised", "Prometido"),
    ("Public key", "Clave pública"),
    ("Put it back in service", "Volver a ponerlo en servicio"),
    ("Recorded when this node joined — what it said about itself when it arrived. Instructions do not travel over the overlay: that node collects them over the same connection it enrolled through, which is why nothing here has to be able to reach it.", "Anotado cuando este nodo se unió — lo que dijo de sí mismo al llegar. Las instrucciones no viajan por la overlay: ese nodo las recoge por la misma conexión con la que se enroló, que es por lo que nada de aquí tiene que poder alcanzarlo."),
    ("Set a domain on this node's own page. A joining node reaches it over the same hostname and certificate this console is served on, and that certificate has to be one the other machine already trusts.", "Ponle un dominio a este nodo en su propia página. El nodo que se une llega por el mismo nombre y el mismo certificado con los que se sirve esta consola, y ese certificado tiene que ser uno en el que la otra máquina ya confíe."),
    ("Swap", "Swap"),
    ("Take it out of service", "Sacarlo de servicio"),
    ("Takes instructions from", "Toma instrucciones de"),
    ("The console, the edge and the deploy path — this process.", "La consola, el edge y el camino de despliegue — este proceso."),
    ("The container runtime, shared by every service.", "El runtime de contenedores, compartido por todos los servicios."),
    ("The images pulled to this node and the snapshots unpacked from them. Nothing collects these yet.", "Las imágenes descargadas a este nodo y los snapshots que se desempaquetaron de ellas. Todavía no hay nada que las recoja."),
    ("The kernel, the distribution, and anything else on this machine.", "El kernel, la distribución y todo lo demás que haya en esta máquina."),
    ("The other machine has to trust the certificate this console is served on. Until this node has a public one, joining will refuse rather than send its token to whatever answered.", "La otra máquina tiene que confiar en el certificado con el que se sirve esta consola. Hasta que este nodo tenga uno público, unirse se negará en vez de mandarle su token a lo que sea que respondió."),
    ("The parts overlap slightly: a container's page cache counts both in its own reading and in the system's, and shared pages count for each process that maps them. \"Everything else\" is what is left over rather than a measurement of its own.", "Las partes se solapan un poco: la caché de páginas de un contenedor cuenta en su propia lectura y en la del sistema, y las páginas compartidas cuentan por cada proceso que las mapea. «Todo lo demás» es lo que sobra, no una medida propia."),
    ("This mints a token carrying this node's address, its overlay key and an address on the overlay for the new node. Joining with it there records this node as an authority — which that node can revoke at any time — and tells this one it arrived.", "Esto acuña un token que lleva la dirección de este nodo, su clave overlay y una dirección en la overlay para el nodo nuevo. Unirse con él allí anota este nodo como autoridad — que ese nodo puede revocar cuando quiera — y le dice a este que llegó."),
    ("This node dials it at", "Este nodo lo marca en"),
    ("This node has no address another one could call back on, so it cannot enrol anybody yet.", "Este nodo no tiene una dirección a la que otro pueda devolver la llamada, así que todavía no puede enrolar a nadie."),
    ("This node now takes instructions from ", "Este nodo ya toma instrucciones de "),
    ("This node stops listing it. It is one direction only: the other node still holds this one as an authority until somebody revokes it there, from its own console. A grant belongs to the node that made it.", "Este nodo deja de listarlo. Va en una sola dirección: el otro nodo sigue teniendo a este como autoridad hasta que alguien lo revoque allí, desde su propia consola. Una concesión es del nodo que la hizo."),
    ("This node's own: its database, its certificates and the logs it keeps for each container.", "Lo propio de este nodo: su base de datos, sus certificados y los logs que guarda de cada contenedor."),
    ("This node, and the ones that have agreed to take instructions from it.", "Este nodo, y los que han aceptado tomar instrucciones de él."),
    ("Tokens this node minted", "Tokens que acuñó este nodo"),
    ("Untick anything you would rather not agree to. You can revoke any of it later from this page, and this node keeps working either way.", "Desmarca lo que prefieras no aceptar. Puedes revocar cualquier parte más tarde desde esta página, y este nodo sigue funcionando igual."),
    ("Waiting to be collected", "Esperando a ser recogido"),
    ("What a service may take is also what is reserved for it: this node counts what it has promised and refuses to promise more. What it keeps for itself — the console, the edge, containerd — is already out of these figures.", "Lo que un servicio puede llevarse es también lo que se le reserva: este nodo cuenta lo que ha prometido y se niega a prometer más. Lo que guarda para sí — la consola, el edge, containerd — ya está fuera de estas cifras."),
    ("What services keep. One disk per copy — two copies of a database on one machine are two databases.", "Lo que guardan los servicios. Un disco por copia — dos copias de una base de datos en una máquina son dos bases de datos."),
    ("What the images themselves are using, from their cgroups.", "Lo que usan las imágenes mismas, leído de sus cgroups."),
    ("answers for names", "responde por nombres"),
    ("joined · ", "unido · "),
    ("runs containers only", "solo ejecuta contenedores"),
    ("the box in the rack by the window", "la máquina del rack junto a la ventana"),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
