use std::fmt;

use serde::Serialize;

use crate::domain::errors::{PlatformError, PlatformResult};

const LONGEST: usize = 8192;

/// The words a container runs, split the way a shell would and run by none.
///
/// No shell is involved: `$HOME` reaches the program as five characters.
/// Expanding it here would be a second, different shell whose rules nobody
/// could look up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CommandLine(Vec<String>);

impl CommandLine {
    /// Empty is `None`, which is the image's own.
    ///
    /// A line starting with `[` is read as a Dockerfile's exec form, because
    /// that is what somebody copying a `CMD` out of one pastes.
    pub fn parse(typed: &str) -> PlatformResult<Option<Self>> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Ok(None);
        }
        if typed.len() > LONGEST {
            return Err(PlatformError::Refused(format!(
                "a command is at most {LONGEST} characters"
            )));
        }
        let words = match typed.starts_with('[') {
            true => serde_json::from_str::<Vec<String>>(typed).map_err(|_| {
                PlatformError::Refused(
                    "that starts like a list and is not one — write [\"node\", \"server.js\"], \
                     or the words without brackets"
                        .into(),
                )
            })?,
            false => shlex::split(typed).ok_or_else(|| {
                PlatformError::Refused(
                    "a quote is opened and never closed, or the line ends in a backslash".into(),
                )
            })?,
        };
        Self::from_words(words).map(Some)
    }

    /// The same rules for words that arrived already split, from a row or
    /// from another node.
    pub fn from_words(words: Vec<String>) -> PlatformResult<Self> {
        match words.first() {
            None => {
                return Err(PlatformError::Refused(
                    "there is nothing to run in that".into(),
                ))
            }
            Some(program) if program.is_empty() => {
                return Err(PlatformError::Refused(
                    "the first word is the program to run, and it is empty".into(),
                ))
            }
            Some(_) => {}
        }
        if words.iter().any(|word| word.contains('\0')) {
            return Err(PlatformError::Refused(
                "a command cannot carry a NUL byte".into(),
            ));
        }
        if words.iter().map(String::len).sum::<usize>() > LONGEST {
            return Err(PlatformError::Refused(format!(
                "a command is at most {LONGEST} characters"
            )));
        }
        Ok(Self(words))
    }

    pub fn words(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for CommandLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match shlex::try_join(self.0.iter().map(String::as_str)) {
            Ok(line) => f.write_str(&line),
            Err(_) => f.write_str(&serde_json::to_string(&self.0).unwrap_or_default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(typed: &str) -> Vec<String> {
        CommandLine::parse(typed)
            .expect("parses")
            .expect("not empty")
            .words()
            .to_vec()
    }

    #[test]
    fn nothing_typed_is_the_images_own() {
        assert_eq!(CommandLine::parse("").expect("empty"), None);
        assert_eq!(CommandLine::parse("   \n ").expect("blank"), None);
    }

    #[test]
    fn quotes_keep_a_word_together_and_nothing_is_expanded() {
        assert_eq!(
            words(r#"sh -c 'echo "$HOME" && sleep 1'"#),
            ["sh", "-c", r#"echo "$HOME" && sleep 1"#]
        );
        assert_eq!(
            words(r#"node worker.js --queue "high priority""#),
            ["node", "worker.js", "--queue", "high priority"]
        );
    }

    #[test]
    fn a_dockerfiles_exec_form_is_read_as_it_is_written() {
        assert_eq!(
            words(r#"["npm", "run", "start:prod"]"#),
            ["npm", "run", "start:prod"]
        );
    }

    #[test]
    fn what_cannot_run_is_refused_with_a_reason() {
        for (typed, reason) in [
            ("node 'server.js", "never closed"),
            (r#"["node", 3]"#, "list"),
            ("[]", "nothing to run"),
            ("'' server.js", "program"),
            ("# only a comment", "nothing to run"),
        ] {
            let refused = CommandLine::parse(typed).expect_err(typed).to_string();
            assert!(refused.contains(reason), "{typed}: {refused}");
        }
        assert!(CommandLine::from_words(vec!["a\0b".into()]).is_err());
        assert!(CommandLine::parse(&"x ".repeat(LONGEST)).is_err());
    }

    /// The form shows what is stored, and saving it unchanged has to store
    /// the same words — or every save of an untouched form would change the
    /// command a little.
    #[test]
    fn what_the_form_shows_reads_back_as_the_same_words() {
        for original in [
            vec!["node", "server.js"],
            vec!["sh", "-c", "echo \"$HOME\" && exec app --flag='x y'"],
            vec!["python", "-c", "print('it\\'s')"],
            vec!["/bin/tool", "", "trailing space ", "tab\there", "new\nline"],
            vec!["#not-a-comment", "~", "*", "a;b", "{c}"],
        ] {
            let line = CommandLine::from_words(original.iter().map(|w| w.to_string()).collect())
                .expect("valid");
            assert_eq!(words(&line.to_string()), original, "{line}");
        }
    }
}
