//! Which images of a service a node keeps.

use std::collections::{HashMap, HashSet};

/// How many images of one service a node keeps when nobody has said.
pub const KEPT_BY_DEFAULT: usize = 2;

/// What a copy's row says when the image it was given could not be fetched
/// and the one it was running still is.
pub fn kept_running(reason: &str) -> String {
    format!("{KEPT_OPENS}{reason}")
}

/// Whether a copy's reason is that note rather than a failure.
pub fn is_kept_running(reason: &str) -> bool {
    reason.starts_with(KEPT_OPENS)
}

const KEPT_OPENS: &str = "still running the image before, because ";

/// One image record, as containerd lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub reference: String,
    pub digest: String,
    /// When the record last changed, in milliseconds.
    pub updated_at: i64,
    /// The manifests the record's target lists, when it is an index.
    pub children: Vec<String>,
}

/// What one collection takes away.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Surplus {
    /// The records to forget.
    pub forget: Vec<String>,
    /// The images that go with them, so the rows naming them can go too.
    pub dropped: Vec<String>,
}

/// The images of one repository beyond `keep`, and every record holding them.
///
/// An image is a record no other record lists: a push leaves a record for
/// each platform manifest beside the index's, and those belong to the index.
/// An image is dated by its release where it has one and by its record where
/// it has not, because a node running a copy has no releases.
///
/// What is in use stays whatever the number, and so does the newest: it is
/// what a push just delivered, and nothing has deployed it yet.
pub fn surplus(
    held: &[Held],
    pushed: &HashMap<String, i64>,
    in_use: &HashSet<String>,
    keep: usize,
) -> Surplus {
    let listed: HashSet<&str> = held
        .iter()
        .flat_map(|record| record.children.iter().map(String::as_str))
        .collect();

    let mut ages: HashMap<&str, i64> = HashMap::new();
    for record in held
        .iter()
        .filter(|record| !listed.contains(record.digest.as_str()))
    {
        let age = pushed
            .get(&record.digest)
            .copied()
            .unwrap_or(record.updated_at);
        let known = ages.entry(record.digest.as_str()).or_insert(age);
        *known = (*known).max(age);
    }
    let mut newest_first: Vec<(&str, i64)> = ages.into_iter().collect();
    newest_first.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    let mut kept: HashSet<&str> = newest_first
        .iter()
        .map(|(digest, _)| *digest)
        .filter(|digest| in_use.contains(*digest))
        .collect();
    kept.extend(newest_first.first().map(|(digest, _)| *digest));
    for (digest, _) in &newest_first {
        if kept.len() >= keep {
            break;
        }
        kept.insert(digest);
    }

    let mut keeping: HashSet<&str> = kept;
    keeping.extend(in_use.iter().map(String::as_str));
    let listed_by_kept: Vec<&str> = held
        .iter()
        .filter(|record| keeping.contains(record.digest.as_str()))
        .flat_map(|record| record.children.iter().map(String::as_str))
        .collect();
    keeping.extend(listed_by_kept);

    Surplus {
        forget: held
            .iter()
            .filter(|record| !keeping.contains(record.digest.as_str()))
            .map(|record| record.reference.clone())
            .collect(),
        dropped: newest_first
            .iter()
            .filter(|(digest, _)| !keeping.contains(*digest))
            .map(|(digest, _)| digest.to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "acr.example/nubo/app";

    fn tagged(tag: &str, digest: &str, at: i64, children: &[&str]) -> Held {
        Held {
            reference: format!("{REPO}:{tag}"),
            digest: digest.into(),
            updated_at: at,
            children: children.iter().map(|child| child.to_string()).collect(),
        }
    }

    fn pinned(digest: &str, at: i64, children: &[&str]) -> Held {
        Held {
            reference: format!("{REPO}@{digest}"),
            digest: digest.into(),
            updated_at: at,
            children: children.iter().map(|child| child.to_string()).collect(),
        }
    }

    /// What a CI push leaves: the index under its tag and under its digest,
    /// and a record for each manifest the index lists.
    fn pushed(name: &str, at: i64) -> Vec<Held> {
        let index = format!("sha256:{name}");
        let platform = format!("sha256:{name}-amd64");
        let attestation = format!("sha256:{name}-attestation");
        vec![
            tagged(name, &index, at, &[&platform, &attestation]),
            pinned(&index, at, &[&platform, &attestation]),
            pinned(&platform, at, &[]),
            pinned(&attestation, at, &[]),
        ]
    }

    fn in_use(digests: &[&str]) -> HashSet<String> {
        digests.iter().map(|digest| digest.to_string()).collect()
    }

    #[test]
    fn the_running_image_and_the_newest_stay_and_the_rest_go_with_every_record() {
        let held: Vec<Held> = [("v1", 1), ("v2", 2), ("v3", 3), ("v4", 4)]
            .into_iter()
            .flat_map(|(name, at)| pushed(name, at))
            .collect();

        let surplus = surplus(&held, &HashMap::new(), &in_use(&["sha256:v2"]), 2);

        assert_eq!(surplus.dropped, ["sha256:v3", "sha256:v1"]);
        let mut forgotten = surplus.forget.clone();
        forgotten.sort();
        assert_eq!(
            forgotten,
            [
                format!("{REPO}:v1"),
                format!("{REPO}:v3"),
                format!("{REPO}@sha256:v1"),
                format!("{REPO}@sha256:v1-amd64"),
                format!("{REPO}@sha256:v1-attestation"),
                format!("{REPO}@sha256:v3"),
                format!("{REPO}@sha256:v3-amd64"),
                format!("{REPO}@sha256:v3-attestation"),
            ]
        );
    }

    #[test]
    fn the_newest_is_kept_even_when_only_one_image_is() {
        let held: Vec<Held> = [("v1", 1), ("v2", 2)]
            .into_iter()
            .flat_map(|(name, at)| pushed(name, at))
            .collect();

        let surplus = surplus(&held, &HashMap::new(), &in_use(&["sha256:v1"]), 1);

        assert!(surplus.dropped.is_empty(), "{surplus:?}");
        assert!(surplus.forget.is_empty(), "{surplus:?}");
    }

    #[test]
    fn fewer_images_than_kept_lose_nothing() {
        let held = pushed("v1", 1);
        for keep in 0..=3 {
            assert_eq!(
                surplus(&held, &HashMap::new(), &HashSet::new(), keep),
                Surplus::default(),
                "{keep}"
            );
        }
    }

    /// Two builds of the same source can share a platform manifest and differ
    /// only in their attestation, so a manifest an older index lists can
    /// still be the newer one's.
    #[test]
    fn a_manifest_a_kept_index_lists_stays_with_it() {
        let held = vec![
            pinned("sha256:old", 1, &["sha256:shared", "sha256:old-att"]),
            pinned("sha256:mid", 2, &["sha256:mid-amd64"]),
            pinned("sha256:new", 3, &["sha256:shared", "sha256:new-att"]),
            pinned("sha256:shared", 1, &[]),
            pinned("sha256:old-att", 1, &[]),
            pinned("sha256:mid-amd64", 2, &[]),
            pinned("sha256:new-att", 3, &[]),
        ];

        let surplus = surplus(&held, &HashMap::new(), &in_use(&["sha256:mid"]), 2);

        assert_eq!(surplus.dropped, ["sha256:old"]);
        assert_eq!(
            surplus.forget,
            [
                format!("{REPO}@sha256:old"),
                format!("{REPO}@sha256:old-att")
            ]
        );
    }

    /// A release dates an image by its push. The record of a tag moves with
    /// every push, so its time is the newest push's, not its own image's.
    #[test]
    fn a_release_dates_its_image_before_the_record_does() {
        let held = vec![
            pinned("sha256:a", 5, &[]),
            pinned("sha256:b", 9, &[]),
            pinned("sha256:c", 7, &[]),
        ];
        let dates = HashMap::from([("sha256:a".to_string(), 10), ("sha256:b".to_string(), 1)]);

        let surplus = surplus(&held, &dates, &HashSet::new(), 2);

        assert_eq!(surplus.dropped, ["sha256:b"]);
    }

    /// A copy pinned to one platform's manifest keeps that manifest, even
    /// when the index that listed it goes.
    #[test]
    fn an_image_in_use_stays_whatever_it_is() {
        let held = vec![
            pinned("sha256:old", 1, &["sha256:old-amd64"]),
            pinned("sha256:old-amd64", 1, &[]),
            pinned("sha256:mid", 2, &[]),
            pinned("sha256:new", 3, &[]),
        ];

        let surplus = surplus(&held, &HashMap::new(), &in_use(&["sha256:old-amd64"]), 2);

        assert_eq!(surplus.dropped, ["sha256:old"]);
        assert_eq!(surplus.forget, [format!("{REPO}@sha256:old")]);
    }
}
