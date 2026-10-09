//! What the Backup tab says: where a copy goes and when, how long it is
//! kept, the bucket's credential, and how the last one went.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (" copied while running", " copiados mientras corrían"),
    (" volume(s)", " volumen(es)"),
    (", never in this node's database: a key that can read every backup in the network must not be inside the thing being backed up. A backup holds the database, the volumes and the images — it does not hold that file.", ", nunca en la base de datos de este nodo: una clave que puede leer todos los backups de la red no debe estar dentro de lo que se respalda. Un backup contiene la base de datos, los volúmenes y las imágenes — no contiene ese fichero."),
    ("A directory on this machine, ssh://[user@]host/path, or s3://bucket/prefix. Leave it empty and the copy stays here, in the path the field shows — which protects against nothing that has ever happened to a disk.", "Un directorio de esta máquina, ssh://[usuario@]host/ruta, o s3://bucket/prefijo. Si lo dejas vacío la copia se queda aquí, en la ruta que muestra el campo — y eso no protege de nada de lo que le haya pasado nunca a un disco."),
    ("A slot gets one attempt: a copy of every database retried every few minutes is a machine that stops answering. Fix what it names and press Back up now, or wait for the next one.", "Cada franja tiene un intento: una copia de cada base de datos repetida cada pocos minutos es una máquina que deja de responder. Arregla lo que dice y pulsa Hacer un backup ahora, o espera a la siguiente."),
    ("Access key id", "Access key id"),
    ("Back up now", "Hacer un backup ahora"),
    ("Backing up this node", "Backup de este nodo"),
    ("Backing up…", "Haciendo el backup…"),
    ("Bucket credentials", "Credenciales del bucket"),
    ("Day", "Día"),
    ("Days. Everything inside the window costs disk and everything outside it is gone — an hourly pass drops the backups that have expired, and the log no surviving backup needs. The oldest one is kept until a newer one can anchor the window, so it is never shorter than it says.", "Días. Todo lo que cae dentro de la ventana cuesta disco y todo lo que queda fuera se ha ido — una pasada cada hora quita los backups caducados y el log que ningún backup superviviente necesita. El más antiguo se guarda hasta que otro más nuevo pueda anclar la ventana, así que nunca es más corta de lo que dice."),
    ("Empty for Amazon's own S3. For anything else, the URL the provider gives you — and with one set, buckets are addressed as a path rather than a subdomain, which is what MinIO and most of the others want.", "Vacío para el S3 de Amazon. Para cualquier otro, la URL que da el proveedor — y con una puesta, los buckets se direccionan como ruta y no como subdominio, que es lo que quieren MinIO y casi todos los demás."),
    ("Endpoint", "Endpoint"),
    ("Every day", "Cada día"),
    ("Every week", "Cada semana"),
    ("For any storage that speaks the S3 protocol — Amazon, Backblaze, Wasabi, DigitalOcean, MinIO, Ceph. They are kept in ", "Para cualquier almacenamiento que hable el protocolo S3 — Amazon, Backblaze, Wasabi, DigitalOcean, MinIO, Ceph. Se guardan en "),
    ("Forget them", "Olvidarlas"),
    ("Friday", "Viernes"),
    ("How often", "Cada cuánto"),
    ("It held", "Contenía"),
    ("Keep the write-ahead log", "Guardar el log de escritura anticipada"),
    ("Kept for", "Se guardan"),
    ("Last attempt", "Último intento"),
    ("Last good one", "Último que funcionó"),
    ("Monday", "Lunes"),
    ("Never — I take them myself", "Nunca — los hago yo"),
    ("Next", "Siguiente"),
    ("Nothing on this machine. A backup taken here and sent somewhere else leaves nothing behind, which is the point — this list is what is still on this disk.", "Nada en esta máquina. Un backup hecho aquí y enviado a otro sitio no deja nada detrás, que es de lo que se trata — esta lista es lo que sigue en este disco."),
    ("Part of the signature, so a wrong one is a refused request rather than a slow one. Providers that have no regions still want the one they document — often us-east-1.", "Forma parte de la firma, así que una región equivocada es una petición rechazada y no una lenta. Los proveedores que no tienen regiones piden igualmente la que documentan — a menudo us-east-1."),
    ("Region", "Región"),
    ("Removing them takes the section out of the file. Nothing else about the plan changes — and backups to a bucket stop working, which is why this is refused while the destination is one.", "Quitarlas borra la sección del fichero. Nada más del plan cambia — y los backups a un bucket dejan de funcionar, que es por lo que esto se rechaza mientras el destino sea uno."),
    ("SSH uses this machine's own keys and needs nothing stored. S3 needs its credentials in config.toml, which is deliberate: a key that can read every backup in the network must not be inside the thing being backed up.", "SSH usa las claves de esta máquina y no necesita guardar nada. S3 necesita sus credenciales en config.toml, y es deliberado: una clave que puede leer todos los backups de la red no debe estar dentro de lo que se respalda."),
    ("Saturday", "Sábado"),
    ("Saving asks the bucket to list what it holds, when the destination above is one — the same call a backup starts with, so a key that saves is a key that works.", "Al guardar se le pide al bucket que liste lo que tiene, si el destino de arriba es uno — la misma llamada con la que empieza un backup, así que una clave que se guarda es una clave que funciona."),
    ("Secret access key", "Secret access key"),
    ("Showing the newest ", "Mostrando los más nuevos: "),
    ("Stored. Leave this empty to keep it — it is never shown again, here or anywhere else.", "Guardado. Déjalo vacío para conservarlo — no se vuelve a mostrar, ni aquí ni en ningún sitio."),
    ("Sunday", "Domingo"),
    ("The copies are sent away and the local staging directory is removed when the transfer succeeds, so this list is not what the destination holds.", "Las copias se envían fuera y el directorio local de paso se borra cuando la transferencia funciona, así que esta lista no es lo que tiene el destino."),
    ("The last attempt did not work: ", "El último intento no funcionó: "),
    ("Thursday", "Jueves"),
    ("Tuesday", "Martes"),
    ("UTC, which is what the node keeps. An hour chosen in your own zone would be somebody else's afternoon, and a copy of every database is not what a busy hour needs.", "UTC, que es lo que lleva el nodo. Una hora elegida en tu propia zona sería la tarde de otra persona, y una copia de cada base de datos no es lo que una hora ocupada necesita."),
    ("Wednesday", "Miércoles"),
    ("Whatever the provider generated. It is written to the file above and never rendered back.", "Lo que haya generado el proveedor. Se escribe en el fichero de arriba y no se vuelve a mostrar."),
    ("Where the copy goes", "Dónde va la copia"),
    ("a node with no id", "un nodo sin id"),
    ("never from here", "nunca desde aquí"),
    ("none on this node", "ninguna en este nodo"),
    ("nothing is scheduled", "no hay nada programado"),
    ("running now", "en marcha ahora"),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
