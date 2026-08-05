//! The HTTP surface.
//!
//! A controller is an impl block. Path, query and body are merged into
//! one request struct and validated before the handler runs, so a
//! handler only ever sees input that already passed its own rules.

use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use wabot::prelude::*;
use wabot::rest::axum::Router;
use wabot::rest::{RestError, RestResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub text: String,
}

/// Built once and shared. Swap the `RwLock` for a repository when you
/// outgrow it — see the `wabot-rust-persistence` skill.
#[singleton]
#[derive(Default)]
pub struct Notes {
    items: RwLock<Vec<Note>>,
}

impl Notes {
    fn all(&self) -> Vec<Note> {
        self.items.read().clone()
    }

    fn add(&self, text: String) -> Note {
        let mut items = self.items.write();
        let note = Note {
            id: format!("n-{}", items.len() + 1),
            text,
        };
        items.push(note.clone());
        note
    }

    fn find(&self, id: &str) -> Option<Note> {
        self.items.read().iter().find(|n| n.id == id).cloned()
    }
}

#[derive(Debug, Deserialize, Validate)]
pub struct GetNote {
    #[description("note id, from the `:id` path segment")]
    #[is_not_empty]
    pub id: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateNote {
    #[description("what the note says")]
    #[is_not_empty]
    #[max_length(280)]
    pub text: String,
}

#[singleton]
pub struct NotesController {
    notes: Arc<Notes>,
}

#[rest_controller("/api/notes")]
impl NotesController {
    #[get("/")]
    async fn list(&self) -> RestResult<Vec<Note>> {
        Ok(self.notes.all())
    }

    #[get("/:id")]
    async fn get_one(&self, req: GetNote) -> RestResult<Note> {
        self.notes
            .find(&req.id)
            .ok_or_else(|| RestError::NotFound(req.id))
    }

    #[post("/")]
    async fn create(&self, req: CreateNote) -> RestResult<Note> {
        Ok(self.notes.add(req.text))
    }
}

/// Nothing is discovered: a type you forget here panics on resolve.
pub fn register(container: &Container) {
    register_singletons!(container, Notes, NotesController);
}

pub fn routes(container: &Container) -> Router {
    NotesController::register_routes(container, Router::new())
}
