//! One file per page. See `docs/architecture-rules.md` §5.
//!
//! A page takes what it needs and returns markup. It does not query and
//! it does not decide.

pub mod guest_join_page;
pub mod guest_setup_page;
pub mod guest_sign_in_page;
pub mod new_database_page;
pub mod new_project_page;
pub mod new_service_page;
pub mod node_user_invite_node_page;
pub mod node_user_join_network_page;
pub mod node_user_network_page;
pub mod node_user_node_settings_page;
pub mod node_user_organisations_page;
pub mod node_user_people_page;
pub mod node_user_release_page;
pub mod node_user_remote_node_page;
pub mod node_user_this_node_page;
pub mod node_user_updates_page;
pub mod project_page;
pub mod project_settings_page;
pub mod projects_page;
pub mod service_logs_page;
pub mod service_page;
pub mod service_settings_page;
pub mod tenant_user_people_page;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    /// **A controller does not draw.** It reads the request, calls the
    /// use case and hands the result to a page — rule 5.
    ///
    /// There were seventy-one `rsx!` blocks in `src/console/`, and the
    /// last nine were the ones worth arguing about: three of them ran to
    /// three hundred lines and read twenty-five locals each, so moving
    /// them as they stood would have enshrined a twenty-five field
    /// struct. What they turned out to be was **tabbed** — one panel per
    /// tab, each reading a fifth of it — which is also why the handlers
    /// now fetch per tab instead of fetching everything for whichever one
    /// was asked for.
    ///
    /// It reads the directory rather than a list of files. *A guard that
    /// names its inputs is right until somebody adds an input*, and the
    /// log-writer's cost guard proved it again in the same afternoon: it
    /// named `console/services.rs` and went red when the card it checks
    /// moved to a page.
    #[test]
    fn a_controller_draws_nothing() {
        let mut drawing: BTreeSet<String> = BTreeSet::new();
        let mut read = 0;
        for entry in fs::read_dir("src/console").expect("the console is there") {
            let path = entry.expect("entry").path();
            if path.extension().is_none_or(|kind| kind != "rs") {
                continue;
            }
            read += 1;
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let source = fs::read_to_string(&path).expect("read");
            if source.contains("rsx! {") {
                drawing.insert(name.to_string());
            }
        }
        assert!(
            read > 8,
            "the scan read {read} controllers, so it is looking in the wrong place"
        );
        assert!(
            drawing.is_empty(),
            "markup belongs in src/views: {drawing:#?}"
        );
    }
}
