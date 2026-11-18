use crate::domain::values::command_line::CommandLine;

/// What a service's container runs, as Docker's two halves. `None` is the
/// image's own.
pub struct SetServiceProcessRequest {
    pub entrypoint: Option<CommandLine>,
    pub command: Option<CommandLine>,
}
