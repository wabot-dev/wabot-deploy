use hypertext::prelude::*;

use crate::infra::deploy::logs::{stamp_of, BOUNDARY};

/// Byte for byte the file's text, so what is copied out of the panel is what
/// the container wrote; only the elements around it change.
pub fn log_text(text: &str) -> impl Renderable + '_ {
    rsx! {
        @for (index, line) in text.split_inclusive('\n').enumerate() {
            @let (words, end) = line.split_at(line.trim_end_matches('\n').len());
            @if words.starts_with(BOUNDARY) && index == 0 {
                <span class="log-run log-run-first">(words)</span>(end)
            } @else if words.starts_with(BOUNDARY) {
                <span class="log-run">(words)</span>(end)
            } @else if let Some(stamp) = stamp_of(line) {
                <span class="log-stamp">(stamp)</span>(&line[stamp.len()..])
            } @else {
                (line)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(text: &str) -> String {
        log_text(text).render().into_inner()
    }

    fn without_tags(html: &str) -> String {
        let mut text = String::new();
        let mut inside = false;
        for c in html.chars() {
            match c {
                '<' => inside = true,
                '>' => inside = false,
                _ if !inside => text.push(c),
                _ => {}
            }
        }
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
    }

    const TWO_RUNS: &str = "── wabot-deploy ── started 2026-11-21 10:00:00 UTC ──\n\
        listening on :8080\n\
        panic: <boom> & \"gone\"\n\
        ── wabot-deploy ── started 2026-11-21 10:00:10 UTC ──\n\
        listening on :8080";

    #[test]
    fn each_run_starts_on_a_line_of_its_own_and_the_first_needs_no_space_above() {
        let html = rendered(TWO_RUNS);

        assert_eq!(html.matches("class=\"log-run\"").count(), 1, "{html}");
        assert_eq!(
            html.matches("class=\"log-run log-run-first\"").count(),
            1,
            "{html}"
        );
        assert!(
            html.contains("<span class=\"log-run\">── wabot-deploy ── started 2026-11-21 10:00:10 UTC ──</span>\n"),
            "{html}"
        );
    }

    #[test]
    fn the_panel_reads_exactly_what_the_file_holds() {
        assert_eq!(without_tags(&rendered(TWO_RUNS)), TWO_RUNS);

        let stamped = "2026-11-21T10:00:00Z stdout ready\n2026-11-21T10:00:01Z stderr <oops>\n";
        assert_eq!(without_tags(&rendered(stamped)), stamped);
    }

    #[test]
    fn the_writer_s_stamp_is_quieter_than_what_the_container_said() {
        let html = rendered("2026-11-21T10:00:01Z stderr disk full\n");

        assert!(
            html.starts_with(
                "<span class=\"log-stamp\">2026-11-21T10:00:01Z stderr </span>disk full"
            ),
            "{html}"
        );
    }
}
