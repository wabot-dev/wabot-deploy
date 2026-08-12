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
    ("Add", "Añadir"),
    ("Answer for hostnames", "Responder por nombres"),
    ("Ask it to run this", "Pedirle que lo ejecute"),
    ("Asked", "Pedido"),
    ("Back to project", "Volver al proyecto"),
    ("Backup", "Copia de seguridad"),
    ("Before you join", "Antes de unirte"),
    ("Can", "Puede"),
    ("Cancel", "Cancelar"),
    ("Certificate on the way", "Certificado en camino"),
    ("Check again", "Comprobar de nuevo"),
    ("Collected", "Recogido"),
    ("Container", "Contenedor"),
    ("Deploy this", "Desplegar esta"),
    ("Done", "Hecho"),
    ("Evicted there", "Expulsada allí"),
    ("Failed", "Falló"),
    ("Finished", "Terminada"),
    ("Forget this node", "Olvidar este nodo"),
    ("How many", "Cuántas"),
    ("Install", "Instalar"),
    ("Install this release", "Instalar esta versión"),
    ("Installed", "Instalada"),
    ("Installing", "Instalando"),
    ("Invite a node", "Invitar un nodo"),
    ("Join", "Unirse"),
    ("Join token", "Token de unión"),
    ("Last update", "Última actualización"),
    ("New ones on", "Las nuevas en"),
    ("Node", "Nodo"),
    ("Nodes", "Nodos"),
    ("Not running", "No está corriendo"),
    ("Now", "Ahora"),
    ("Overview", "Resumen"),
    ("People", "Personas"),
    ("Person", "Persona"),
    ("Project", "Proyecto"),
    ("Projects", "Proyectos"),
    ("Push tokens", "Tokens de subida"),
    ("Reachable at", "Se llega en"),
    ("Refused", "Rechazado"),
    ("Releases", "Versiones"),
    ("Remove", "Quitar"),
    ("Replica", "Réplica"),
    ("Restarting", "Reiniciando"),
    ("Run containers", "Ejecutar contenedores"),
    ("Running", "Corriendo"),
    ("Save", "Guardar"),
    ("Save placement", "Guardar colocación"),
    ("Served by", "Servido por"),
    ("Serving", "Sirviendo"),
    ("Settings", "Ajustes"),
    ("Sign out", "Cerrar sesión"),
    ("Started", "Empezada"),
    ("State", "Estado"),
    ("Updates", "Actualizaciones"),
    ("Username", "Usuario"),
    ("Version", "Versión"),
    ("Waiting to be collected", "Esperando a ser recogido"),
    ("What this node does", "Qué hace este nodo"),
    ("What to call it", "Cómo llamarlo"),
    ("Where this runs", "Dónde corre"),
    ("You are: ", "Tu rol: "),
    ("answers for names", "responde por nombres"),
    ("runs containers only", "solo ejecuta contenedores"),
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

    #[test]
    fn nothing_is_translated_to_itself_or_to_nothing() {
        for (english, spanish) in TABLE {
            assert!(!spanish.is_empty(), "{english:?} has no Spanish");
            assert_ne!(english, spanish, "{english:?} is not translated");
        }
    }

    #[test]
    fn a_string_that_is_there_is_found() {
        assert_eq!(lookup("Projects"), Some("Proyectos"));
        assert_eq!(lookup("nothing like this"), None);
    }
}
