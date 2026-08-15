//! The Spanish for what the console says.
//!
//! Long and mechanical on purpose, and kept away from
//! [`super::language`], which is neither. Sorted by the English, which
//! is what makes the lookup a binary search and what makes a duplicate
//! or a stray entry something `cargo test` can find rather than
//! something a reader has to.
//!
//! ## What is not in here
//!
//! Anything a machine reads or a person typed: hostnames, ids, image
//! names, slugs, and the words containerd uses for a container's state.
//! Translating those would make a page that cannot be pasted into a
//! terminal or an issue.
//!
//! ## Voice
//!
//! The same voice as the English: plain, second person, no exclamation
//! marks, and the impersonal *se* over *usted*. Where English says "This
//! node has no address", Spanish says "Este nodo no tiene dirección" —
//! not "Usted no ha configurado". The console describes the machine, it
//! does not address the reader about their conduct.

/// Every string, `(english, spanish)`, sorted by the English.
///
/// Sorted because the lookup below binary-searches it, and because a
/// list a thousand entries long is only maintainable if there is one
/// place a given string can be.
pub(crate) const TABLE: &[(&str, &str)] = &[
    (" free", " libres"),
    (" here", " aquí"),
    (" is available.", " está disponible."),
    (" of ", " de "),
    (" values · ", " valores · "),
    (" will not resolve. Either add *.<node domain> pointing at this node, or type a hostname you have already pointed here — it is checked before it is accepted.", " no va a resolver. Añade *.<dominio del nodo> apuntando a este nodo, o escribe un nombre que ya hayas apuntado aquí — se comprueba antes de aceptarlo."),
    (" · pre-release", " · versión previa"),
    (" · published ", " · publicada "),
    (" · this node", " · este nodo"),
    (
        " — reserved for it, and not something a certificate can vouch for.",
        " — reservada para ella, y no algo por lo que un certificado pueda responder.",
    ),
    (" — same name, same certificate.", " — mismo nombre, mismo certificado."),
    (
        " — the same name with -ro in its first label. Changing this reissues the certificate and rewrites the names inside every container of this project.",
        " — el mismo nombre con -ro en su primera etiqueta. Cambiarlo reemite el certificado y reescribe los nombres dentro de cada contenedor de este proyecto.",
    ),
    ("A ceiling on the container and the engine's own settings, together. Postgres is given a quarter of it for shared buffers and told to expect half of it as cache — its defaults alone would be killed on the smaller sizes.", "Un techo para el contenedor y los ajustes del propio motor, a la vez. A Postgres se le da un cuarto para los buffers compartidos y se le dice que espere la mitad como caché — sus valores por defecto solos morirían en los tamaños pequeños."),
    (
        "A copy here is measured every couple of seconds; one on another node is measured by that node, across the interval it reports on. The same unit, and the second is the smoother of the two.",
        "Una copia de aquí se mide cada par de segundos; una de otro nodo la mide ese nodo, a lo largo del intervalo con el que reporta. La misma unidad, y la segunda es la más suave de las dos.",
    ),
    (
        "A copy writes its log on the machine that runs it, and this node cannot read another one's disk. Open the console of the node holding it.",
        "Una copia escribe su log en la máquina que la ejecuta, y este nodo no puede leer el disco de otra. Abre la consola del nodo que la tiene.",
    ),
    (
        "A database is reached by name, from inside the project. The node writes these into every container it starts, so nothing in an image has to be configured.",
        "A una base de datos se llega por nombre, desde dentro del proyecto. El nodo los escribe en cada contenedor que arranca, así que no hay nada que configurar en una imagen.",
    ),
    ("A key and an address appear the first time this node enrols another one or joins one itself. The overlay is what carries traffic between nodes — an edge here reaching a container that runs somewhere else.", "La clave y la dirección aparecen la primera vez que este nodo enrola a otro o se une a uno. La overlay es lo que lleva el tráfico entre nodos — un edge de aquí llegando a un contenedor que corre en otro sitio."),
    ("A new copy is created on the node you pick, rather than here and moved after — which would start a container on this machine and stop it again for nothing. Removing takes the ones already thrown out first, then the highest-numbered; the node running one is told to stop it.", "Una copia nueva se crea en el nodo que elijas, en vez de aquí y moverla después — lo que arrancaría un contenedor en esta máquina para pararlo acto seguido sin motivo. Al quitar se van primero las ya expulsadas, luego las de número más alto; al nodo que corre una se le dice que la pare."),
    (
        "A node answers for a name by claiming it, getting a certificate for it, and proxying to wherever the copies run. Only nodes with an address the world can dial are here, and only those that agreed to be asked.",
        "Un nodo responde por un nombre reclamándolo, consiguiendo un certificado para él y haciendo de proxy hacia donde corran las copias. Aquí solo están los nodos con una dirección a la que el mundo pueda llamar, y solo los que aceptaron que se les pida.",
    ),
    ("A note, if you want one", "Una nota, si quieres"),
    (
        "A public authority signs it, so any client verifies it with the trust store it already has — the connection string needs no certificate of its own.",
        "Lo firma una autoridad pública, así que cualquier cliente lo verifica con el almacén de confianza que ya trae — la cadena de conexión no necesita certificado propio.",
    ),
    ("A reference containerd can resolve. Fully qualified — there is no implicit registry here.", "Una referencia que containerd pueda resolver. Completa — aquí no hay registry implícito."),
    ("A wildcard record covers this node, so this name already resolves here. Leave it as it is.", "Un registro comodín cubre este nodo, así que este nombre ya resuelve aquí. Déjalo como está."),
    ("Accounts", "Cuentas"),
    ("Add", "Añadir"),
    ("Add port", "Añadir puerto"),
    ("Address", "Dirección"),
    ("Administrator — everything", "Administrador — todo"),
    ("All nodes", "Todos los nodos"),
    ("Allowed", "Permitido"),
    ("Already thrown out. Its containers are stopped and the node that placed it has been told — or will be, the next time it asks.", "Ya expulsada. Sus contenedores están parados y el nodo que la colocó ya lo sabe — o lo sabrá, la próxima vez que pregunte."),
    ("And offer it, in return:", "Y ofrecerle, a cambio:"),
    ("Answer for hostnames", "Responder por nombres"),
    ("Answer for its hostnames from this node", "Responder por sus nombres desde este nodo"),
    ("Answer for this node's hostnames", "Responder por los nombres de este nodo"),
    ("As", "Como"),
    ("Ask it to run this", "Pedirle que lo ejecute"),
    ("Ask that node to let this one:", "Pedirle a ese nodo que deje a este:"),
    ("Ask whoever invited you for another.", "Pídele otra a quien te invitó."),
    ("Ask whoever runs this node to set one.", "Pídesela a quien lleva este nodo."),
    ("Asked", "Pedido"),
    ("At least 12 characters. A phrase beats a puzzle — this console can start containers on the machine.", "Doce caracteres como mínimo. Una frase vale más que un acertijo — esta consola puede arrancar contenedores en la máquina."),
    ("At least 12 characters. Nobody here will ever see it.", "Doce caracteres como mínimo. Aquí nadie va a verla nunca."),
    ("Back to", "Volver a"),
    ("Back to project", "Volver al proyecto"),
    ("Back to service", "Volver al servicio"),
    ("Backup", "Copia de seguridad"),
    ("Before you join", "Antes de unirte"),
    ("Both are read now and refused if they do not match, do not cover this name, or have already expired — a bad pair installed would break every handshake, including the one serving this page. After that the node rereads them and reinstalls whatever it finds, which is how a certificate it cannot renew stays current.", "Se leen ahora los dos y se rechazan si no casan, si no cubren este nombre o si ya caducaron — un par malo instalado rompería cada handshake, incluido el que sirve esta página. Después el nodo los relee y reinstala lo que encuentre, que es como sigue al día un certificado que no puede renovar."),
    ("Both lists travel inside the token and are shown on the other machine before it is spent. Whoever holds it accepts or refuses each one — so what is asked for here is a request, not a setting.", "Las dos listas viajan dentro del token y se muestran en la otra máquina antes de gastarlo. Quien lo tenga acepta o rechaza cada una — así que lo que se pide aquí es una petición, no un ajuste."),
    (
        "Both names resolve in every container of this project, on any node holding a copy. The long one is the same string everywhere and the only form a certificate authority could sign; the short one means nothing outside this project.",
        "Los dos nombres resuelven en cada contenedor de este proyecto, en cualquier nodo que sostenga una copia. El largo es la misma cadena en todas partes y la única forma que una autoridad de certificación podría firmar; el corto no significa nada fuera de este proyecto.",
    ),
    (
        "Both resolve inside this project, on any node holding a copy. Neither reaches the database from outside the node — that is a published port, which is not built.",
        "Las dos resuelven dentro de este proyecto, en cualquier nodo que sostenga una copia. Ninguna alcanza la base desde fuera del nodo — eso es publicar un puerto, que no está construido.",
    ),
    ("Breadcrumb", "Ruta de navegación"),
    ("CPU", "CPU"),
    ("Can", "Puede"),
    ("Cancel", "Cancelar"),
    ("Certificate", "Certificado"),
    ("Certificate file", "Fichero del certificado"),
    ("Certificate on the way", "Certificado en camino"),
    ("Certificates", "Certificados"),
    ("Check again", "Comprobar de nuevo"),
    ("Collected", "Recogido"),
    ("Connection string", "Cadena de conexión"),
    ("Connections", "Conexiones"),
    ("Container", "Contenedor"),
    ("Container port", "Puerto del contenedor"),
    ("Copied", "Copiada"),
    ("Copies", "Copias"),
    ("Copy", "Copiar"),
    ("Copy ", "Copia "),
    ("Could not read the release list: ", "No se pudo leer la lista de versiones: "),
    ("Create account", "Crear cuenta"),
    ("Create administrator", "Crear administrador"),
    ("Create database", "Crear base de datos"),
    ("Create invitation", "Crear invitación"),
    ("Create join token", "Crear token de unión"),
    ("Create project", "Crear proyecto"),
    ("Create service", "Crear servicio"),
    ("Create token", "Crear token"),
    ("Danger zone", "Zona de peligro"),
    ("Database", "Base de datos"),
    ("Database name", "Nombre de la base"),
    ("Delete database and its data", "Eliminar la base de datos y sus datos"),
    ("Delete project", "Borrar proyecto"),
    ("Delete service", "Borrar servicio"),
    ("Deleting a database stops it and removes everything it stored on this node. There is no undo and there is no backup — a read-only copy is not one, because a deletion reaches it too.", "Eliminar una base de datos la para y borra todo lo que guardó en este nodo. No hay vuelta atrás y no hay copia de seguridad — una réplica de solo lectura no lo es, porque el borrado también llega hasta ella."),
    ("Deleting a project deletes every service under it. Nothing is stopped first — do that yourself.", "Borrar un proyecto borra todos sus servicios. No se para nada antes — eso hazlo tú."),
    ("Deleting a service stops its container and removes it. The images it was built from stay in the registry.", "Borrar un servicio para su contenedor y lo quita. Las imágenes con las que se construyó siguen en el registry."),
    ("Deploy a push automatically", "Desplegar automáticamente al subir"),
    ("Deploy this", "Desplegar esta"),
    ("Deploying", "Desplegando"),
    ("Disk", "Disco"),
    ("Domain", "Dominio"),
    ("Done", "Hecho"),
    ("Download", "Descarga"),
    ("Download the authority", "Descargar la autoridad"),
    ("Earlier attempts", "Intentos anteriores"),
    ("Environment", "Entorno"),
    ("Error: ", "Error: "),
    (
        "Every container in this project resolves that name, on any node holding a copy — whether or not anything is exposed.",
        "Cada contenedor de este proyecto resuelve ese nombre, en cualquier nodo que sostenga una copia — se exponga algo o no.",
    ),
    ("Evicted there", "Expulsada allí"),
    ("Expired", "Caducada"),
    ("Failed", "Falló"),
    ("Finished", "Terminada"),
    ("Following", "Siguiendo"),
    ("For", "Para"),
    ("For a database or anything that is not HTTP. The node picks the outside port. It is reachable from the whole internet unless a firewall says otherwise.", "Para una base de datos o cualquier cosa que no sea HTTP. El nodo elige el puerto de fuera. Se llega desde todo internet salvo que un cortafuegos diga lo contrario."),
    (
        "For any container in this project, on any node holding a copy. It names the database rather than an address, so it survives a redeployment and reads the same everywhere.",
        "Para cualquier contenedor de este proyecto, en cualquier nodo que sostenga una copia. Nombra la base de datos en lugar de una dirección, así que sobrevive a un redespliegue y se lee igual en todas partes.",
    ),
    (
        "For anything that is not HTTP — a queue, a socket, an engine with its own protocol. The node picks the outside port, and it is reachable from the whole internet unless a firewall says otherwise.",
        "Para cualquier cosa que no sea HTTP — una cola, un socket, un motor con su propio protocolo. El nodo elige el puerto de fuera, y se llega desde todo internet salvo que un cortafuegos diga lo contrario.",
    ),
    ("Forget", "Olvidar"),
    ("Forget this node", "Olvidar este nodo"),
    ("From", "De"),
    ("From another node", "De otro nodo"),
    ("From outside the node", "Desde fuera del nodo"),
    (
        "From outside the node, neither resolves — and there is no edge to choose: an edge terminates TLS and proxies HTTP, while Postgres speaks its own protocol with TLS inside the server. The domain is the node's, inherited by every database it owns.",
        "Desde fuera del nodo no resuelve ninguno — y no hay edge que elegir: un edge termina TLS y proxya HTTP, mientras Postgres habla su propio protocolo con TLS dentro del servidor. El dominio es el del nodo, heredado por toda base que posea.",
    ),
    (
        "From the directory with the Dockerfile, after docker login with a push token from the project page.",
        "Desde el directorio con el Dockerfile, después de docker login con un token de subida de la página del proyecto.",
    ),
    ("From the nodes page of the node you are joining: invite a node there, and it shows one token, once. Pasting it here does what `wabot-deploy join` does in a terminal — this node records that one as an authority and tells it so. The same token can be pasted again if something goes wrong part-way.", "Desde la página de nodos del nodo al que te unes: invita un nodo allí, y enseña un token, una vez. Pegarlo aquí hace lo que hace `wabot-deploy join` en una terminal — este nodo anota aquel como autoridad y se lo dice. El mismo token se puede volver a pegar si algo sale mal a medias."),
    ("Have it answer for this node's hostnames", "Que responda por los nombres de este nodo"),
    ("Have this node answer for its hostnames", "Que este nodo responda por sus nombres"),
    ("How many", "Cuántas"),
    ("How to push", "Cómo subir una imagen"),
    ("Id", "Id"),
    ("Image", "Imagen"),
    ("Inside the project", "Dentro del proyecto"),
    ("Install", "Instalar"),
    ("Install this release", "Instalar esta versión"),
    ("Installed", "Instalada"),
    ("Installing", "Instalando"),
    ("Into a project", "En un proyecto"),
    ("Invitations", "Invitaciones"),
    ("Invite a node", "Invitar un nodo"),
    ("Invite somebody", "Invitar a alguien"),
    ("Issuer", "Emisor"),
    ("It can ask this node to terminate TLS for one of its names and proxy to wherever that service runs.", "Puede pedirle a este nodo que termine TLS para uno de sus nombres y haga de proxy hacia donde corra ese servicio."),
    ("It can place a read-only copy of one of its databases here, and that copy is every row of it — on this node's disk, taking this node's space. Running somebody's container is not the same favour, which is why this is asked for separately.", "Puede colocar aquí una copia de solo lectura de una de sus bases de datos, y esa copia es cada fila de ella — en el disco de este nodo, ocupando su espacio. Correr el contenedor de alguien no es el mismo favor, y por eso esto se pide aparte."),
    ("It can place replicas of its services here and pull the images for them. It cannot touch anything else on this node.", "Puede colocar aquí réplicas de sus servicios y traerse las imágenes para ellas. No puede tocar nada más de este nodo."),
    ("It has no address yet. A connection string appears once it has been deployed.", "Todavía no tiene dirección. La cadena de conexión aparece cuando se haya desplegado."),
    ("It must resolve to this node, and this node must be reachable on port 80 — that is what the challenge answers on. Both are checked before anything is requested. Saving reissues: the node takes the new name on its own certificate straight away, then asks a public authority for one.", "Tiene que resolver a este nodo, y este nodo tiene que ser alcanzable en el puerto 80 — que es donde responde el reto. Se comprueban las dos cosas antes de pedir nada. Guardar reemite: el nodo toma el nombre nuevo en su propio certificado de inmediato, y luego se lo pide a una autoridad pública."),
    ("It terminates TLS for names, its own and other nodes'. Turning this off makes it a private node — that is all private has ever meant.", "Termina TLS para nombres, los suyos y los de otros nodos. Apagar esto lo convierte en un nodo privado — que es lo único que privado ha significado nunca."),
    ("It works once and expires in 24 hours. This node will not show it again — what is stored is its hash. The other machine has to be a node already: install it there first, then join.", "Funciona una vez y caduca en 24 horas. Este nodo no volverá a mostrarlo — lo que se guarda es su hash. La otra máquina tiene que ser ya un nodo: instálalo allí primero, y luego únelo."),
    ("It works once and expires in seven days. The node will not show it again — it is not stored in a form anybody can read back.", "Funciona una vez y caduca en siete días. El nodo no volverá a mostrarla — no se guarda de una forma que nadie pueda leer."),
    ("Its domain", "Su dominio"),
    ("Join", "Unirse"),
    ("Join a network", "Unirse a una red"),
    ("Join token", "Token de unión"),
    ("Joined", "Unido"),
    ("Keep a copy of its data on this node", "Guardar una copia de sus datos en este nodo"),
    ("Keep a copy of this node's data over there", "Guardar allí una copia de los datos de este nodo"),
    ("Key file", "Fichero de la clave"),
    ("Last update", "Última actualización"),
    ("Logs", "Logs"),
    ("Long name", "Nombre largo"),
    ("Make a member", "Hacer miembro"),
    ("Make an administrator", "Hacer administrador"),
    ("May", "Puede"),
    ("Member — their own projects", "Miembro — sus propios proyectos"),
    ("Memory", "Memoria"),
    ("Name", "Nombre"),
    ("Names", "Nombres"),
    ("Needs a domain that resolves here. Without an address the world can dial, this node is private whatever it would prefer.", "Necesita un dominio que resuelva aquí. Sin una dirección que el mundo pueda marcar, este nodo es privado quiera o no."),
    ("New", "Nuevo"),
    ("New ones on", "Las nuevas en"),
    ("No build here", "Sin binario aquí"),
    (
        "No copy of this service runs on this node.",
        "Ninguna copia de este servicio corre en este nodo.",
    ),
    ("No projects yet.", "Todavía no hay proyectos."),
    ("No services yet.", "Todavía no hay servicios."),
    ("No setup token is outstanding, so nobody can be created from here. Issue one on the node:", "No hay ningún token de instalación pendiente, así que no se puede crear a nadie desde aquí. Emite uno en el nodo:"),
    ("No wildcard record answers for this node, so ", "Ningún registro comodín responde por este nodo, así que "),
    ("Nobody yet. Administrators of this node reach every project without being added to it.", "Todavía nadie. Los administradores de este nodo llegan a todos los proyectos sin estar añadidos a ellos."),
    ("Node", "Nodo"),
    ("Nodes", "Nodos"),
    ("Nodes this one has granted authority to. Revoking is one row, and it takes effect here — a node that has been revoked can ask for nothing.", "Nodos a los que este ha concedido autoridad. Revocar es una fila, y surte efecto aquí — un nodo revocado no puede pedir nada."),
    ("None.", "Ninguno."),
    ("None. A token is what CI authenticates with — it is nobody's password, and revoking it changes nothing else.", "Ninguno. Un token es con lo que se autentica CI — no es la contraseña de nadie, y revocarlo no cambia nada más."),
    ("Not deployed", "Sin despliegue"),
    ("Not following — reload to see more", "Sin seguir — recarga para ver más"),
    ("Not published", "Sin publicar"),
    ("Not running", "No está corriendo"),
    (
        "Nothing answers at any of these while it is not running.",
        "Mientras no esté corriendo, en ninguna de estas responde nada.",
    ),
    ("Nothing has been pushed yet. Create a push token on the project page and push an image to this repository.", "Todavía no se ha subido nada. Crea un token de subida en la página del proyecto y sube una imagen a este repositorio."),
    ("Nothing yet", "Todavía nada"),
    (
        "Nothing yet. A container that has only just started may not have written anything.",
        "Nada todavía. Un contenedor que acaba de arrancar puede no haber escrito nada.",
    ),
    (
        "Nothing, and there is no edge to choose: an edge terminates TLS and proxies HTTP, while Postgres speaks its own protocol with TLS inside the server. Reaching one from outside is a published port, which is not built yet.",
        "Nada, y no hay edge que elegir: un edge termina TLS y proxya HTTP, mientras Postgres habla su propio protocolo con TLS dentro del servidor. Alcanzarla desde fuera es publicar un puerto, que todavía no está construido.",
    ),
    (
        "Nothing. It answers inside the project only — publish a port or give one a hostname in settings.",
        "Nada. Sólo responde dentro del proyecto — publica un puerto o dale un nombre a uno en ajustes.",
    ),
    ("Nothing. It is handing this node an address on its overlay and asking for nothing back.", "Nada. Le está dando a este nodo una dirección en su overlay y no pide nada a cambio."),
    ("Nothing. This node will take its instructions and give it none.", "Nada. Este nodo tomará sus instrucciones y no le dará ninguna."),
    ("Now", "Ahora"),
    ("On this network", "En esta red"),
    ("On this node", "En este nodo"),
    ("One KEY=value per line. Everything after the first = is the value, so a value may contain one.", "Un CLAVE=valor por línea. Todo lo que va tras el primer = es el valor, así que un valor puede llevar uno."),
    ("One per hostname. A node with no public DNS, or a name a certificate authority cannot reach, is what the other two answers are for.", "Uno por nombre. Un nodo sin DNS público, o un nombre al que una autoridad de certificación no llega, es para lo que están las otras dos respuestas."),
    (
        "One per running container. The runtime's overhead, not the image's.",
        "Uno por contenedor en marcha. El coste del runtime, no el de la imagen.",
    ),
    (
        "Only to tell one unspent token from another. The node names itself when it joins, and that \
         is the name this console shows.",
        "Sólo para distinguir un token sin gastar de otro. El nodo se nombra a sí mismo al unirse, \
         y ese es el nombre que muestra esta consola.",
    ),
    ("Only what it was granted, which is listed beside it below. Revoking any of it takes effect here and immediately.", "Solo lo que se le concedió, que está listado a su lado abajo. Revocar cualquier parte surte efecto aquí y de inmediato."),
    ("Open", "Abrir"),
    ("Outcome", "Resultado"),
    ("Output", "Salida"),
    ("Overlay address", "Dirección overlay"),
    ("Overview", "Resumen"),
    ("Password", "Contraseña"),
    ("People", "Personas"),
    ("Person", "Persona"),
    ("Placed by", "Colocada por"),
    ("Ports", "Puertos"),
    ("Primary", "Primaria"),
    ("Primary — reads and writes", "Primaria — lecturas y escrituras"),
    ("Private", "Privada"),
    ("Project", "Proyecto"),
    ("Projects", "Proyectos"),
    ("Public", "Pública"),
    ("Public key", "Clave pública"),
    ("Publish", "Publicar"),
    ("Publish on the node's public address (raw TCP)", "Publicar en la dirección pública del nodo (TCP en crudo)"),
    ("Published", "Publicada"),
    ("Push tokens", "Tokens de subida"),
    ("Queued for that node", "En cola para ese nodo"),
    ("Reachable at", "Se llega en"),
    ("Reachable from any container in this project. The address is reserved for this copy, so it survives a redeployment.", "Alcanzable desde cualquier contenedor de este proyecto. La dirección está reservada para esta copia, así que sobrevive a un redespliegue."),
    ("Reaches", "Alcanza"),
    ("Read it on GitHub", "Leerlo en GitHub"),
    ("Read pool", "Pool de lectura"),
    ("Read pool — refuses writes", "Pool de lectura — rechaza escrituras"),
    ("Reconnecting…", "Reconectando…"),
    ("Recorded when this node joined — what it said about itself when it arrived. Instructions do not travel over the overlay: that node collects them over the same connection it enrolled through, which is why nothing here has to be able to reach it.", "Anotado cuando este nodo se unió — lo que dijo de sí mismo al llegar. Las instrucciones no viajan por la overlay: ese nodo las recoge por la misma conexión con la que se enroló, que es por lo que nada de aquí tiene que poder alcanzarlo."),
    ("Refused", "Rechazado"),
    ("Releases", "Versiones"),
    ("Remove", "Quitar"),
    ("Renews in", "Se renueva en"),
    ("Replica", "Réplica"),
    ("Replicas can be placed here — its own services included. A small node can own projects and run none of them.", "Se pueden colocar réplicas aquí — las de sus propios servicios incluidas. Un nodo pequeño puede tener proyectos y no ejecutar ninguno."),
    (
        "Resolves in every container of this project, on any node holding a copy — the long name in the world's DNS too. Neither reaches the database from outside the node: that is a published port, which is not built.",
        "Resuelve en cada contenedor de este proyecto, en cualquier nodo que sostenga una copia — el nombre largo también en el DNS del mundo. Ninguno alcanza la base desde fuera del nodo: eso es publicar un puerto, que no está construido.",
    ),
    ("Restarting", "Reiniciando"),
    ("Restore", "Restaurar"),
    ("Revoke", "Revocar"),
    ("Revoked", "Revocado"),
    ("Run a service there", "Ejecutar un servicio allí"),
    ("Run containers", "Ejecutar contenedores"),
    ("Run its containers on this node", "Ejecutar sus contenedores en este nodo"),
    ("Run this node's containers over there", "Correr allí los contenedores de este nodo"),
    ("Run this node's containers there", "Ejecutar allí los contenedores de este nodo"),
    ("Run this on the other node", "Ejecutar esto en el otro nodo"),
    ("Running", "Corriendo"),
    ("Running elsewhere", "Corriendo en otro nodo"),
    ("Running here", "Corriendo aquí"),
    ("Save", "Guardar"),
    ("Save domain", "Guardar dominio"),
    ("Save environment", "Guardar entorno"),
    ("Save name", "Guardar nombre"),
    ("Save placement", "Guardar colocación"),
    ("Save source", "Guardar origen"),
    ("Saving redeploys the service with these values. The image it runs does not change.", "Guardar redespliega el servicio con estos valores. La imagen que ejecuta no cambia."),
    ("Send this link", "Envía este enlace"),
    ("Serve over HTTPS at a hostname", "Servir por HTTPS en un nombre"),
    ("Served by", "Servido por"),
    ("Service", "Servicio"),
    ("Serving", "Sirviendo"),
    ("Set a domain on this node's own page. A joining node reaches it over the same hostname and certificate this console is served on, and that certificate has to be one the other machine already trusts.", "Ponle un dominio a este nodo en su propia página. El nodo que se une llega por el mismo nombre y el mismo certificado con los que se sirve esta consola, y ese certificado tiene que ser uno en el que la otra máquina ya confíe."),
    ("Set the node's domain", "Poner el dominio del nodo"),
    ("Settings", "Ajustes"),
    ("Setup token", "Token de instalación"),
    ("Short name", "Nombre corto"),
    ("Shown once. Use it as the password: ", "Se muestra una vez. Úsalo como contraseña: "),
    ("Sign in", "Entrar"),
    ("Sign out", "Cerrar sesión"),
    (
        "Signed by this node, covering every name above. A container verifies it with the authority the node places at /etc/wabot/ca.crt.",
        "Firmado por este nodo, cubriendo todos los nombres de arriba. Un contenedor lo verifica con la autoridad que el nodo coloca en /etc/wabot/ca.crt.",
    ),
    ("Somebody who already has an account on this node — adding them here does not create one. A new person is invited from Settings, People.", "Alguien que ya tiene cuenta en este nodo — añadirla aquí no crea ninguna. A una persona nueva se la invita desde Ajustes, Personas."),
    ("Started", "Empezada"),
    ("State", "Estado"),
    ("Stop publishing", "Dejar de publicar"),
    ("Stops its containers here and tells the node that placed it to stop asking. It cannot be undone from this side — that node decides what happens next, and it may place it somewhere else.", "Para sus contenedores aquí y le dice al nodo que la colocó que deje de pedirla. No se puede deshacer desde este lado — ese nodo decide qué pasa después, y puede colocarla en otro sitio."),
    ("Swap", "Swap"),
    ("Tag to watch", "Etiqueta a vigilar"),
    ("Takes instructions from", "Toma instrucciones de"),
    ("That node writes its own project, its own service row and its own deployment — nothing is shared. It pulls the image from this node's registry with a credential this puts in the instruction, so the image travels only when it is needed.", "Ese nodo escribe su propio proyecto, su propia fila de servicio y su propio despliegue — no se comparte nada. Tira la imagen del registry de este nodo con una credencial que esto mete en la instrucción, así que la imagen viaja solo cuando hace falta."),
    ("The ceiling on the container and the engine's own settings, together. It takes effect at the next deployment: a cgroup limit is written when the container is created, and nothing reaches into a running one to change it.", "El techo del contenedor y los ajustes del propio motor, a la vez. Surte efecto en el siguiente despliegue: el límite del cgroup se escribe al crear el contenedor, y nada entra en uno que ya corre para cambiarlo."),
    (
        "The console, the edge and the deploy path — this process.",
        "La consola, el edge y el camino de despliegue — este proceso.",
    ),
    (
        "The container runtime, shared by every service.",
        "El runtime de contenedores, compartido por todos los servicios.",
    ),
    ("The image is pulled from Docker Hub. The major version is fixed once the database exists: changing it is a data migration, not an image change.", "La imagen se trae de Docker Hub. La versión mayor queda fija en cuanto la base existe: cambiarla es una migración de datos, no un cambio de imagen."),
    (
        "The kernel, the distribution, and anything else on this machine.",
        "El kernel, la distribución y todo lo demás que haya en esta máquina.",
    ),
    ("The machine is yours: throwing it out is something you can always do, and it is the only thing here that is.", "La máquina es tuya: echarlo fuera es algo que siempre puedes hacer, y es lo único aquí que lo es."),
    (
        "The name already resolves from outside and nothing answers there. Publishing opens a port on every interface of this node and maps it to the primary.",
        "El nombre ya resuelve desde fuera y ahí no responde nada. Publicar abre un puerto en cada interfaz de este nodo y lo mapea a la primaria.",
    ),
    (
        "The node chose the port, so it cannot collide with another database's. It survives a redeployment and changes only if you turn this off and on again.",
        "El puerto lo eligió el nodo, así que no puede chocar con el de otra base. Sobrevive a un redespliegue y sólo cambia si lo apagas y lo vuelves a encender.",
    ),
    ("The node restarts on its own when the new binary is in place. This page follows along — there is nothing to reload.", "El nodo se reinicia solo cuando el binario nuevo está en su sitio. Esta página lo sigue — no hay nada que recargar."),
    (
        "The node's, inherited — set a domain on the node and every database it owns is named under it. A copy held on another machine keeps the owner's domain, because the name belongs to the database.",
        "El del nodo, heredado — pon un dominio en el nodo y toda base que posea se nombra bajo él. Una copia sostenida en otra máquina conserva el dominio del dueño, porque el nombre es de la base de datos.",
    ),
    ("The other machine has to trust the certificate this console is served on. Until this node has a public one, joining will refuse rather than send its token to whatever answered.", "La otra máquina tiene que confiar en el certificado con el que se sirve esta consola. Hasta que este nodo tenga uno público, unirse se negará en vez de mandarle su token a lo que sea que respondió."),
    ("The parts overlap slightly: a container's page cache counts both in its own reading and in the system's, and shared pages count for each process that maps them. \"Everything else\" is what is left over rather than a measurement of its own.", "Las partes se solapan un poco: la caché de páginas de un contenedor cuenta en su propia lectura y en la del sistema, y las páginas compartidas cuentan por cada proceso que las mapea. «Todo lo demás» es lo que sobra, no una medida propia."),
    (
        "The pool falls back to the primary while there is no read-only copy, so an application written against it keeps working when the last one is taken away.",
        "El pool cae al primario mientras no haya copia de solo lectura, así que una aplicación escrita contra él sigue funcionando cuando se quita la última.",
    ),
    (
        "The pool holds every read-only copy, and each container is given them in its own order — so ten applications do not all put the same copy first. That is spread rather than balance: one client keeps using the copy it picked.",
        "El pool tiene todas las copias de solo lectura, y a cada contenedor se le dan en su propio orden — así diez aplicaciones no ponen todas la misma copia primero. Eso es reparto, no balanceo: un cliente sigue usando la copia que eligió.",
    ),
    ("The primary — reads and writes", "El primario — lecturas y escrituras"),
    (
        "The push appears above as a release, and waits for somebody to deploy it. Any other tag is stored and changes nothing here.",
        "La subida aparece arriba como versión, y espera a que alguien la despliegue. Cualquier otra etiqueta se guarda y no cambia nada aquí.",
    ),
    (
        "The push deploys it. Any other tag is stored and changes nothing here.",
        "La subida la despliega. Cualquier otra etiqueta se guarda y no cambia nada aquí.",
    ),
    ("The read pool answers at ", "El pool de lectura responde en "),
    ("The read pool — refuses writes", "El pool de lectura — rechaza escrituras"),
    ("The rows stay because they are what carries that news. Removing them here would leave the other node sending the same instruction again.", "Las filas se quedan porque son lo que lleva esa noticia. Quitarlas aquí dejaría al otro nodo mandando la misma instrucción otra vez."),
    ("The setup token was printed by `wabot-deploy install`. It works once, and it expires.", "El token de instalación lo imprimió `wabot-deploy install`. Funciona una vez, y caduca."),
    ("The slug is derived from the name, and it is what hostnames and containerd labels are built from.", "El slug se deriva del nombre, y es con lo que se construyen los nombres de host y las etiquetas de containerd."),
    (
        "These reach it from inside the project, on any node holding a copy — the long name resolves in the world's DNS too, and nothing answers there until a port is published below.",
        "Estas llegan desde dentro del proyecto, en cualquier nodo que sostenga una copia — el nombre largo también resuelve en el DNS del mundo, y ahí no responde nada hasta que se publique un puerto abajo.",
    ),
    (
        "These reach it from inside the project, on any node holding a copy. From outside the node it answers on the published port, ",
        "Estas llegan desde dentro del proyecto, en cualquier nodo que sostenga una copia. Desde fuera del nodo responde en el puerto publicado, ",
    ),
    ("They choose their own username and password. Nobody here ever sees it — which is the reason this is a link rather than a form that sets one for them.", "Elige su propio usuario y su contraseña. Aquí nadie la ve nunca — que es la razón de que esto sea un enlace y no un formulario que se la ponga."),
    (
        "This container was started before its output was being kept. Deploy it again and it will write from then on.",
        "Este contenedor se arrancó antes de que se guardara su salida. Despliégalo otra vez y escribirá desde ese momento.",
    ),
    (
        "This copy is on the project's bridge at ",
        "Esta copia está en el bridge del proyecto en ",
    ),
    ("This invitation is not valid. It may have been used already, withdrawn, or expired — they last seven days.", "Esta invitación no vale. Puede que ya se usara, que se retirara o que caducara — duran siete días."),
    ("This is the newest release published.", "Esta es la versión más nueva publicada."),
    ("This is you", "Este eres tú"),
    ("This mints a token carrying this node's address, its overlay key and an address on the overlay for the new node. Joining with it there records this node as an authority — which that node can revoke at any time — and tells this one it arrived.", "Esto acuña un token que lleva la dirección de este nodo, su clave overlay y una dirección en la overlay para el nodo nuevo. Unirse con él allí anota este nodo como autoridad — que ese nodo puede revocar cuando quiera — y le dice a este que llegó."),
    ("This node dials it at", "Este nodo lo marca en"),
    ("This node has no address another one could call back on, so it cannot enrol anybody yet.", "Este nodo no tiene una dirección a la que otro pueda devolver la llamada, así que todavía no puede enrolar a nadie."),
    ("This node has no domain of its own, so it cannot check that a name points here — and it will not route one it could not check. The node needs a domain before anything can be served over HTTPS.", "Este nodo no tiene dominio propio, así que no puede comprobar que un nombre apunte aquí — y no va a enrutar uno que no pudo comprobar. El nodo necesita un dominio antes de poder servir nada por HTTPS."),
    ("This node has no services to send. Deploy one here first — what travels is an instruction to run the same image, pulled from this node's registry.", "Este nodo no tiene servicios que mandar. Despliega uno aquí primero — lo que viaja es una instrucción de ejecutar la misma imagen, tirada del registry de este nodo."),
    ("This node installs a release when you ask it to, and never on its own. Installing one restarts the node; the containers on it keep running.", "Este nodo instala una versión cuando se lo pides, y nunca por su cuenta. Instalar una reinicia el nodo; los contenedores que tiene siguen corriendo."),
    ("This node now takes instructions from ", "Este nodo ya toma instrucciones de "),
    (
        "This node signs it, so a client has to be given the authority before it can verify anything. A container gets it at /etc/wabot/ca.crt; anything else needs the file.",
        "Lo firma este nodo, así que a un cliente hay que darle la autoridad antes de que pueda verificar nada. Un contenedor la recibe en /etc/wabot/ca.crt; cualquier otra cosa necesita el fichero.",
    ),
    ("This node stops listing it. It is one direction only: the other node still holds this one as an authority until somebody revokes it there, from its own console. A grant belongs to the node that made it.", "Este nodo deja de listarlo. Va en una sola dirección: el otro nodo sigue teniendo a este como autoridad hasta que alguien lo revoque allí, desde su propia consola. Una concesión es del nodo que la hizo."),
    ("This node, and the ones that have agreed to take instructions from it.", "Este nodo, y los que han aceptado tomar instrucciones de él."),
    ("This release came with no notes.", "Esta versión vino sin notas."),
    ("This runs the container. Which nodes answer for the service's name is chosen on the service itself, and can be this node, that one, or both.", "Esto ejecuta el contenedor. Qué nodos responden por el nombre del servicio se elige en el propio servicio, y puede ser este nodo, aquel, o los dos."),
    ("This service exposes nothing. That is the right answer for a worker; a port is added in settings.", "Este servicio no expone nada. Esa es la respuesta correcta para un worker; un puerto se añade en ajustes."),
    ("This service exposes nothing. That is the right answer for a worker; add a port for anything that listens.", "Este servicio no expone nada. Esa es la respuesta correcta para un worker; añade un puerto para cualquier cosa que escuche."),
    ("This service is administered from the node that placed it here, and nothing on this page will change it.", "Este servicio se administra desde el nodo que lo colocó aquí, y nada de esta página lo va a cambiar."),
    ("This service is not running anywhere.", "Este servicio no corre en ningún sitio."),
    ("Throw it off this node", "Echarlo de este nodo"),
    ("Throw it out", "Echarlo fuera"),
    ("Tokens this node minted", "Tokens que acuñó este nodo"),
    ("Try again", "Reintentar"),
    ("Unknown", "Desconocido"),
    ("Untick anything you would rather not agree to. You can revoke any of it later from this page, and this node keeps working either way.", "Desmarca lo que prefieras no aceptar. Puedes revocar cualquier parte más tarde desde esta página, y este nodo sigue funcionando igual."),
    ("Updates", "Actualizaciones"),
    ("Used", "Usada"),
    ("User", "Usuario"),
    ("Username", "Usuario"),
    ("Version", "Versión"),
    ("Waiting", "Esperando"),
    ("Waiting for that node", "Esperando a ese nodo"),
    ("Waiting to be collected", "Esperando a ser recogido"),
    ("What is in it", "Qué lleva"),
    (
        "What the images themselves are using, from their cgroups.",
        "Lo que usan las imágenes mismas, leído de sus cgroups.",
    ),
    ("What the process listens on inside the container.", "En qué escucha el proceso dentro del contenedor."),
    (
        "What this copy has written since it started. The file is emptied on every deployment, so this is the current attempt and not a history.",
        "Lo que esta copia ha escrito desde que arrancó. El fichero se vacía en cada despliegue, así que esto es el intento actual y no un historial.",
    ),
    ("What this node does", "Qué hace este nodo"),
    ("When", "Cuándo"),
    ("Where the certificate comes from", "De dónde viene el certificado"),
    ("Where this runs", "Dónde corre"),
    ("Who answers for these names", "Quién responde por estos nombres"),
    ("Withdraw", "Retirar"),
    ("Without this, a push is recorded as a release and waits for somebody to deploy it from the service page.", "Sin esto, una subida se anota como versión y espera a que alguien la despliegue desde la página del servicio."),
    ("You are: ", "Tu rol: "),
    ("You were invited as ", "Te invitaron como "),
    ("answers for names", "responde por nombres"),
    ("everything else", "todo lo demás"),
    ("joined · ", "unido · "),
    ("never used", "sin usar"),
    ("newer than what is running", "más nueva que la que corre"),
    ("no build for this machine", "no hay binario para esta máquina"),
    ("no ceiling", "sin techo"),
    ("nothing", "nada"),
    ("older than what is running", "más vieja que la que corre"),
    ("running here", "corriendo aquí"),
    ("runs containers only", "solo ejecuta contenedores"),
    (
        "the box in the rack by the window",
        "la máquina del rack junto a la ventana",
    ),
    ("this node", "este nodo"),
    ("used", "usada"),
];

/// The Spanish for one English string.
///
/// Binary search rather than a `HashMap`: the table is static and
/// sorted, so this needs no allocation, no lazy initialisation and no
/// hashing — and a page is a few hundred lookups, not a few million.
pub(crate) fn lookup(english: &str) -> Option<&'static str> {
    TABLE
        .binary_search_by(|(key, _)| (*key).cmp(english))
        .ok()
        .map(|at| TABLE[at].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lookup binary-searches, so an unsorted table silently stops
    /// finding things — the worst possible failure here, because the
    /// page still renders and simply reverts to English.
    #[test]
    fn the_table_is_sorted_and_has_no_duplicates() {
        for pair in TABLE.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "out of order or duplicated: {:?} then {:?}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    /// A word that is the same in both languages, listed rather than
    /// allowed silently. Otherwise "translated to itself" cannot tell a
    /// deliberate `Swap` from a line somebody pasted and forgot.
    const THE_SAME_IN_BOTH: &[&str] = &["CPU", "Error: ", "Id", "Logs", "Swap"];

    #[test]
    fn nothing_is_left_untranslated_by_accident() {
        for (english, spanish) in TABLE {
            assert!(!spanish.is_empty(), "{english:?} has no Spanish");
            if english == spanish {
                assert!(
                    THE_SAME_IN_BOTH.contains(english),
                    "{english:?} is not translated — add the Spanish, or say here \
                     that it is the same word in both"
                );
            }
        }
    }

    /// Every `t("…")` in the console has Spanish behind it.
    ///
    /// The source is read at compile time and scanned, which is the only
    /// way to make "you translated everything" mechanical: a missing
    /// entry does not fail, it quietly renders one line in English on a
    /// page that is otherwise Spanish, and nobody notices until somebody
    /// reading Spanish does.
    ///
    /// A string added to a page and not to the table fails here, by
    /// name. That is the point.
    #[test]
    fn every_string_the_console_asks_for_is_translated() {
        const SOURCES: &[&str] = &[
            include_str!("shell.rs"),
            include_str!("layout.rs"),
            include_str!("auth.rs"),
            include_str!("people.rs"),
            include_str!("databases.rs"),
            include_str!("projects.rs"),
            include_str!("services.rs"),
            include_str!("nodes.rs"),
            include_str!("updates.rs"),
        ];

        let mut missing = Vec::new();
        for source in SOURCES {
            for asked in calls(source) {
                if lookup(&asked).is_none() {
                    missing.push(asked);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "no Spanish for {} string(s): {missing:#?}",
            missing.len()
        );
    }

    /// Every string that reaches `t` through a *variable* rather than
    /// as a literal.
    ///
    /// `calls` below scans for `t("…")`, which is what almost every
    /// string is — and it cannot see `t(asking(capability))`, where the
    /// English lives in a `match` somewhere else. Adding a capability
    /// put two new strings on the join screen and the suite stayed
    /// green, which is a test passing for a reason that has nothing to
    /// do with the thing being right.
    ///
    /// So the indirect ones are named here by hand. A list that has to
    /// be maintained is worse than one that does not, and it is a great
    /// deal better than a check that quietly stops covering things.
    #[test]
    fn the_words_that_reach_t_through_a_variable_are_translated_too() {
        use crate::network::capability::Capability;

        let mut missing = Vec::new();
        for capability in Capability::ALL {
            for word in [
                crate::console::nodes::asking(capability),
                crate::console::nodes::why(capability),
                crate::console::nodes::offering(capability),
            ] {
                if lookup(word).is_none() {
                    missing.push(word.to_string());
                }
            }
        }
        assert!(missing.is_empty(), "no Spanish for {missing:#?}");
    }

    /// Every `t("…")` in one file, as the string `t` receives.
    ///
    /// Hand-written rather than a regex crate: this runs in a test and
    /// the shape it looks for is fixed. It has to undo Rust's `\`
    /// line-continuation, because the literal in the source is not what
    /// reaches `t` — the newline and the indentation after it are gone
    /// by then, and the table is keyed by what arrives.
    fn calls(source: &str) -> Vec<String> {
        let bytes = source.as_bytes();
        let mut found = Vec::new();
        let mut at = 0;

        while let Some(offset) = source[at..].find("t(\"") {
            let start = at + offset;
            at = start + 3;
            // `post("…")` and `format!("…")` end in `t(` too, so the
            // character before has to be one that cannot be part of a
            // name.
            if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
                continue;
            }
            let mut literal = String::new();
            let mut chars = source[at..].chars().peekable();
            let mut escaped = false;
            let mut closed = false;
            while let Some(c) = chars.next() {
                if escaped {
                    match c {
                        // A continuation: drop it and the indentation.
                        '\n' => {
                            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                                chars.next();
                            }
                        }
                        '"' => literal.push('"'),
                        '\\' => literal.push('\\'),
                        other => literal.push(other),
                    }
                    escaped = false;
                    continue;
                }
                match c {
                    '\\' => escaped = true,
                    '"' => {
                        closed = true;
                        break;
                    }
                    other => literal.push(other),
                }
            }
            if closed {
                found.push(literal);
            }
        }
        found
    }

    #[test]
    fn a_string_that_is_there_is_found() {
        assert_eq!(lookup("Projects"), Some("Proyectos"));
        assert_eq!(lookup("nothing like this"), None);
    }
}
