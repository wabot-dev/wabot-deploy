//! The frame around every page: the navigation, the theme and the
//! language toggle.
//!
//! Sorted by the English, which is what makes a duplicate or a stray
//! entry something `cargo test` finds rather than something a reader
//! has to.

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("Breadcrumb", "Ruta de navegación"),
    ("Error: ", "Error: "),
    ("Overview", "Resumen"),
    ("Sign out", "Cerrar sesión"),
];
