/// How much somebody should care.
///
/// Two levels, not five. The question this page answers is "do I have to
/// do something now or not", and a scale with a middle is a scale where
/// everything ends up in the middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Weight {
    /// Worth knowing, nothing is failing because of it.
    Notice,
    /// Something is not working, and it will not fix itself.
    Wrong,
}

impl Weight {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Wrong => "badge badge-danger",
            Self::Notice => "badge badge-warning",
        }
    }

    pub fn dot(&self) -> &'static str {
        match self {
            Self::Wrong => "dot dot-danger",
            Self::Notice => "dot dot-warning",
        }
    }
}

/// One thing that needs somebody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcernResponse {
    pub weight: Weight,
    /// What it is, in the fewest words that are still true. Translated
    /// through `language::word`, because it is chosen here and rendered
    /// elsewhere — the same door every badge word goes through.
    pub what: String,
    /// Which thing: a service's name, a hostname, a node's name, a path.
    /// Never translated — it is somebody's own string or the machine's.
    pub which: String,
    /// The reason, as whatever wrote it put it. Also never translated:
    /// it is containerd's words, or an authority's, or a filesystem's.
    pub why: String,
    /// Where to go and do something about it.
    pub go: String,
}
