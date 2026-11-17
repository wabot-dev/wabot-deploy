//! What a store of buckets' pages say.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    (" objects", " objetos"),
    ("3 to 63 lowercase letters, digits and hyphens, starting and ending with a letter or a digit.", "De 3 a 63 letras minúsculas, dígitos y guiones, empezando y terminando con una letra o un dígito."),
    ("A bucket is deleted only when it is empty, and its keys stop working with it. Emptying one is a client's job: the store refuses while anything is in it.", "Un bucket sólo se borra cuando está vacío, y sus claves dejan de funcionar con él. Vaciarlo es trabajo de un cliente: el almacén se niega mientras quede algo dentro."),
    ("A ceiling on the container. An eighth of it holds what is waiting to reach the disk; the rest is the engine, its metadata and the requests in flight.", "Un techo para el contenedor. Un octavo guarda lo que espera llegar al disco; el resto es el motor, sus metadatos y las peticiones en curso."),
    ("A write that would take it past this is refused with an error the client reads. The store's buckets together have to fit its disk.", "Una escritura que lo pasaría de aquí se rechaza con un error que el cliente lee. Los buckets del almacén juntos tienen que caber en su disco."),
    ("Abandon unfinished uploads after, in days", "Abandonar subidas sin terminar tras, en días"),
    ("Addressing", "Direccionamiento"),
    ("All buckets", "Todos los buckets"),
    ("Anyone with this address reads an object by its key, with no key of their own. It does not list what the bucket holds.", "Cualquiera con esta dirección lee un objeto por su clave, sin una clave de acceso propia. No lista lo que guarda el bucket."),
    ("Create bucket", "Crear bucket"),
    ("Create key", "Crear clave"),
    ("Delete bucket", "Borrar bucket"),
    ("Delete objects after, in days", "Borrar objetos tras, en días"),
    ("Empty is never. An upload in parts that nobody finished holds disk and appears in no listing. The store looks once a day, at midnight UTC, so an object goes within a day of its time, not at it.", "Vacío es nunca. Una subida por partes que nadie terminó ocupa disco y no aparece en ningún listado. El almacén mira una vez al día, a medianoche UTC, así que un objeto se va dentro del día de su plazo, no en el momento exacto."),
    ("Empty is the one shown. It has to point at this node, and the store's allowlist applies to it as to the store's own name. A path that ends in / answers its index.html, and nothing answers a listing.", "Vacío es el que se muestra. Tiene que apuntar a este nodo, y la lista de acceso del almacén se le aplica como al nombre propio del almacén. Una ruta que termina en / responde su index.html, y nada responde un listado."),
    ("How to reach it", "Cómo llegar a él"),
    ("It may", "Puede"),
    ("Keys", "Claves"),
    ("Left on this store's disk: ", "Queda en el disco de este almacén: "),
    ("Make private", "Hacer privado"),
    ("Make public", "Hacer público"),
    ("Most clients address a bucket as a name of its own unless told otherwise. Each has a setting for this:", "La mayoría de los clientes tratan un bucket como un nombre propio salvo que se les diga lo contrario. Cada uno tiene un ajuste para esto:"),
    ("Name it is read at", "Nombre en el que se lee"),
    ("None yet. A bucket holds objects under keys, and is reached with a key of its own.", "Ninguno todavía. Un bucket guarda objetos bajo claves, y se llega a él con una clave propia."),
    ("None. A key opens this bucket and no other, and its secret is shown once, when it is made.", "Ninguna. Una clave abre este bucket y ningún otro, y su secreto se muestra una vez, cuando se crea."),
    ("One origin per line, like https://app.example.com; https://*.example.com for every name under it, or * for any page at all. Without one, an upload from a browser fails there and works in curl.", "Un origen por línea, como https://app.example.com; https://*.example.com para cada nombre bajo él, o * para cualquier página. Sin ninguno, una subida desde un navegador falla allí y funciona con curl."),
    ("Pages that may reach it from a browser", "Páginas que pueden llegar a él desde un navegador"),
    ("Private: every request is signed with one of its keys.", "Privado: cada petición va firmada con una de sus claves."),
    ("Public read", "Lectura pública"),
    ("Read at", "Se lee en"),
    ("Reads and writes", "Lee y escribe"),
    ("Reads only", "Sólo lee"),
    ("Rules", "Reglas"),
    ("The name clients reach it at over HTTPS. Empty is the one shown, with the store's name in place of <name>. It has to point at this node.", "El nombre en el que los clientes llegan a él por HTTPS. Vacío es el que se muestra, con el nombre del almacén en lugar de <name>. Tiene que apuntar a este nodo."),
    ("The name clients reach it at over HTTPS. This node has no domain to derive one under, so it has to be typed, and it has to point at this node.", "El nombre en el que los clientes llegan a él por HTTPS. Este nodo no tiene dominio bajo el que derivar uno, así que hay que escribirlo, y tiene que apuntar a este nodo."),
    ("The store is not running, so what each bucket holds cannot be asked, and keys cannot be made.", "El almacén no está corriendo, así que no se puede preguntar qué guarda cada bucket, ni crear claves."),
    ("The store is not running, so what it holds cannot be asked.", "El almacén no está corriendo, así que no se puede preguntar qué guarda."),
    ("This node runs no stores of buckets. An administrator turns them on in the node's settings, under Capabilities.", "Este nodo no corre almacenes de buckets. Un administrador los activa en los ajustes del nodo, en Capacidades."),
    ("Unfinished uploads", "Subidas sin terminar"),
    ("What it is for", "Para qué es"),
    ("What its buckets may hold together. Every bucket is given a maximum, and their sum has to fit here — a full bucket answers its client with an error rather than filling a disk other services share.", "Lo que sus buckets pueden guardar juntos. Cada bucket recibe un máximo, y su suma tiene que caber aquí — un bucket lleno responde a su cliente con un error en vez de llenar un disco que comparten otros servicios."),
    ("path-style: the bucket is the first part of the path", "path-style: el bucket es la primera parte de la ruta"),
    ("up to ", "hasta "),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}
