//! What the page that shows a secret once says.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("It works once and expires in 24 hours. That machine has to be a node already: install it there first, then join.", "Sirve una vez y caduca en 24 horas. Esa máquina tiene que ser ya un nodo: instálalo allí primero, y luego únelo."),
    ("It works once and expires in seven days. They choose their own username and password.", "Sirve una vez y caduca en siete días. Eligen su propio usuario y contraseña."),
    ("Log in with this", "Entra con esto"),
    ("On the other node's console: Network, then Join a network. Or on its terminal:", "En la consola del otro nodo: Red, y luego Unirse a una red. O en su terminal:"),
    ("Paste this into the other node", "Pega esto en el otro nodo"),
    ("Send this link", "Envía este enlace"),
    ("Shown once", "Se muestra una vez"),
    ("This is the only time it is shown. What is stored is a hash of it, so no page can show it again — copy it now, and make another if you lose it.", "Es la única vez que se muestra. Lo que se guarda es un hash, así que ninguna página puede volver a enseñarlo — cópialo ahora, y haz otro si lo pierdes."),
    ("Use it as the password wherever this project's images are pushed from. It is nobody's password: revoking it changes nothing else.", "Úsalo como contraseña desde donde se suban las imágenes de este proyecto. No es la contraseña de nadie: revocarlo no cambia nada más."),
];
