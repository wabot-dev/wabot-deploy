//! The frame around every page: the navigation, the theme and the
//! language toggle.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("Breadcrumb", "Ruta de navegación"),
    ("Error: ", "Error: "),
    ("Go to", "Ir a"),
    ("Go to…", "Ir a…"),
    ("Nothing here is called that.", "Nada aquí se llama así."),
    ("Overview", "Resumen"),
    ("Sections", "Secciones"),
    ("Sign out", "Cerrar sesión"),
    ("Status", "Estado"),
    (
        "Type the name of something here to go to it.",
        "Escribe aquí el nombre de algo para ir a ello.",
    ),
];
