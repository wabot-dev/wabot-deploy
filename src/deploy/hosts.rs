//! The names a container can reach its neighbours by.
//!
//! ## A file, not a resolver
//!
//! One `/etc/hosts` per container, written from the rows and bind-
//! mounted. No listener, no port 53, and nothing that becomes an open
//! resolver if a binding is ever got wrong.
//!
//! The usual objection to a hosts file is that it goes stale, and here
//! it does not: it is a **bind mount**, so rewriting it on the node
//! changes what a *running* container sees at once, and `getaddrinfo`
//! reads the file on every call rather than caching it. A database
//! created this morning is reachable from an application started last
//! week without redeploying the application.
//!
//! Rewritten **in place** — truncate and write, never write-and-rename.
//! A rename replaces the inode, and a bind mount follows the inode it
//! was made from: the container would keep seeing the old file for ever
//! while the node kept editing a new one nobody had mounted.
//!
//! That is not a worry, it is measured. On a node, `sed -i` on one of
//! these files — which renames, like most editors when they save — left
//! the node holding inode 792415 and the container reading 792419, and
//! the container went on seeing a name that had been deleted. Recreating
//! the container was the only repair. `std::fs::write` truncates the
//! same inode, which is why [`write`] uses it and why nothing here may
//! be "improved" into a write-to-temp-and-rename.
//!
//! ## Scoped to the project
//!
//! A container is given its own project's names and no others. Two
//! projects' bridges are separate L2 domains and `runtime::network`
//! says the isolation is the point of separating them; a name that
//! crossed it would be a hole opened by a naming convention. Reaching
//! another project is something somebody should have to ask for.
//!
//! ## A database's name is its primary
//!
//! Every copy of a web service is interchangeable, so its name lists
//! all of them. A database's copies are not: a standby refuses writes,
//! so one name over primary and standbys fails a share of every
//! application's writes. The primary keeps the name; the standbys get
//! `<name>-ro`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One set of names and the addresses behind them.
///
/// The names are built by [`entries_for`] rather than by the renderer,
/// because the qualified one is not the same for every service on a
/// node: a copy held for somebody else answers to the **owner's**
/// domain, and the renderer has no way to know whose a service is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub names: Vec<String>,
    pub addresses: Vec<String>,
}

/// The suffix a read pool's name takes.
///
/// A suffix rather than a prefix so the two names sort together, and
/// because `orders-ro` reads as "orders, read only" where `ro-orders`
/// reads as a different service.
pub const READ_ONLY: &str = "-ro";

/// The read pool's name, from the primary's.
///
/// `-ro` goes into the **first label**, so `db.example.com` reads
/// `db-ro.example.com`: one name the operator chose governs both, and the
/// pool's is predictable rather than a second thing to keep in step.
///
/// It degenerates to what was there before — `orders.db-test.node.example`
/// becomes `orders-ro.db-test.node.example` — which is the point: nothing
/// moves for a database that never had a name chosen for it.
pub fn pool_name(primary: &str) -> String {
    match primary.split_once('.') {
        Some((first, rest)) => format!("{first}{READ_ONLY}.{rest}"),
        None => format!("{primary}{READ_ONLY}"),
    }
}

/// Where a container's hosts file lives on the node.
pub fn path(data_dir: &Path, container_id: &str) -> PathBuf {
    data_dir.join("hosts").join(container_id)
}

/// The file, as the container will read it.
pub fn render(entries: &[Entry]) -> String {
    let mut file = String::from(
        "# Written by wabot-deploy. Edits are lost at the next change —\n\
         # and editing this by hand is worse than useless: anything that\n\
         # saves by renaming (vim, sed -i) replaces the file the\n\
         # container has mounted, and it stops seeing this one at all.\n\
         #\n\
         # The names of this project, and nothing else: two projects'\n\
         # bridges are separate networks, and a name that crossed that\n\
         # would be a hole opened by a naming convention.\n\
         127.0.0.1\tlocalhost\n\
         ::1\tlocalhost ip6-localhost ip6-loopback\n",
    );

    if entries.is_empty() {
        return file;
    }
    file.push_str("\n# This project.\n");

    // By address, so a name with several copies is several lines rather
    // than one line with several addresses — which is the form every
    // resolver reads the same way.
    for entry in entries {
        for address in &entry.addresses {
            file.push_str(&format!("{address}\t{}\n", entry.names.join(" ")));
        }
    }
    file
}

/// Write one container's file, in place.
pub fn write(data_dir: &Path, container_id: &str, entries: &[Entry]) -> std::io::Result<PathBuf> {
    let path = path(data_dir, container_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // `std::fs::write` truncates and writes the same inode, which is
    // what a bind mount needs — see the module docs.
    std::fs::write(&path, render(entries))?;
    Ok(path)
}

pub fn discard(data_dir: &Path, container_id: &str) {
    let path = path(data_dir, container_id);
    if let Err(error) = std::fs::remove_file(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(file = %path.display(), %error, "removing a hosts file");
        }
    }
}

/// Gather the names of one project from what runs in it.
///
/// `primary_slot` is `Some` for a managed database, and it is what
/// splits the names: that copy keeps the service's name, and every other
/// copy goes into the read pool. `None` — an ordinary service — puts
/// every copy behind the one name, because they are interchangeable and
/// that is the whole point of having several.
///
/// `services` is `(slug, primary slot, qualified domain)`. The domain is
/// per service rather than per node: a database held for somebody else
/// answers to the **owner's** name, and a name built from the holding
/// machine's domain is one no client would write.
///
/// `reader` is who the file is being written for. The read pool is
/// **rotated by it**, so ten application containers do not all put the
/// same replica first — see [`rotate`].
pub fn entries_for(
    services: &[(String, Option<u32>, Option<String>)],
    addresses: &BTreeMap<(String, u32), String>,
    project: &str,
    reader: &str,
) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    // The short names are built; the qualified one is **given**.
    //
    // It used to be built too — slug, project and the node's domain — and
    // that was the same thing only while every name was a subdomain of the
    // node. A database's name is the operator's to choose, so it arrives
    // here as a value and this stops guessing at it.
    let names_for = |slug: &str, qualified: Option<&String>| {
        let mut names = vec![slug.to_string(), format!("{slug}.{project}")];
        if let Some(qualified) = qualified {
            names.push(qualified.clone());
        }
        names
    };

    for (slug, primary_slot, qualified) in services {
        let mut writable = Vec::new();
        let mut readable = Vec::new();
        for ((service, slot), address) in addresses {
            if service != slug {
                continue;
            }
            match primary_slot {
                Some(primary) if slot != primary => readable.push(address.clone()),
                _ => writable.push(address.clone()),
            }
        }

        if !writable.is_empty() {
            entries.push(Entry {
                names: names_for(slug, qualified.as_ref()),
                addresses: writable.clone(),
            });
        }
        // A read pool exists only where there is something in it, and it
        // falls back to the primary: an application written against
        // `orders-ro` should keep working when the last standby is taken
        // away, reading from the only copy there is.
        if primary_slot.is_some() {
            let pool = match readable.is_empty() {
                true => writable,
                false => readable,
            };
            if !pool.is_empty() {
                entries.push(Entry {
                    names: names_for(
                        &format!("{slug}{READ_ONLY}"),
                        qualified.as_ref().map(|name| pool_name(name)).as_ref(),
                    ),
                    addresses: rotate(pool, reader),
                });
            }
        }
    }
    entries
}

/// Put a different member of the pool first for each reader.
///
/// A resolver hands back the addresses of a name in the order it found
/// them, and a client takes the first one that answers. So a pool
/// written in one order is not a pool: every application container in
/// the project reads from the same replica, and the others hold data
/// nobody looks at.
///
/// **This is spread, not balance.** One reader still sends every
/// connection to one replica until that replica stops answering. Real
/// per-connection balancing is `load_balance_hosts=random`, which libpq
/// has done since PostgreSQL 16 and which shuffles exactly these
/// addresses — the node stays out of the data path, and the TLS session
/// runs end to end to whichever replica the client picked.
///
/// The alternative was a proxy of the node's own holding an address on
/// the project's bridge. It would balance per connection, and it would
/// put the node in the path of every query, need an address managed on
/// an interface CNI owns, and know too little about Postgres to health
/// check what it was balancing. That is a phase with its own
/// justification, not a detail of this one.
fn rotate(pool: Vec<String>, reader: &str) -> Vec<String> {
    if pool.len() < 2 {
        return pool;
    }
    // A stable hash of the reader's own name, so the order is the same
    // every time this file is rewritten. Shuffling on each write would
    // move a client's replica underneath it for no reason.
    let seed = reader.bytes().fold(0u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u64)
    });
    let at = (seed % pool.len() as u64) as usize;
    let mut rotated = pool[at..].to_vec();
    rotated.extend_from_slice(&pool[..at]);
    rotated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addresses(pairs: &[(&str, u32, &str)]) -> BTreeMap<(String, u32), String> {
        pairs
            .iter()
            .map(|(service, slot, address)| ((service.to_string(), *slot), address.to_string()))
            .collect()
    }

    /// The rule that keeps writes working. A standby refuses them, so a
    /// name covering both fails a share of every application's writes —
    /// on a node, `cannot execute INSERT in a read-only transaction`.
    #[test]
    fn a_databases_name_is_its_primary_and_the_pool_is_a_second_name() {
        let entries = entries_for(
            &[("orders".to_string(), Some(1), None)],
            &addresses(&[
                ("orders", 1, "10.42.2.200"),
                ("orders", 2, "10.42.2.254"),
                ("orders", 3, "10.42.2.253"),
            ]),
            "demo",
            "demo.web",
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].names[0], "orders");
        assert_eq!(
            entries[0].addresses,
            vec!["10.42.2.200".to_string()],
            "only the primary takes writes"
        );

        assert_eq!(entries[1].names[0], "orders-ro");
        // Every standby and no primary. The order is the reader's — see
        // `rotate` — so this asserts the membership, which is the claim.
        let mut pool = entries[1].addresses.clone();
        pool.sort();
        assert_eq!(pool, vec!["10.42.2.253", "10.42.2.254"]);
    }

    /// An application written against the pool keeps working when the
    /// last standby is taken away: it reads from the only copy there is.
    #[test]
    fn the_read_pool_falls_back_to_the_primary() {
        let entries = entries_for(
            &[("orders".to_string(), Some(1), None)],
            &addresses(&[("orders", 1, "10.42.2.200")]),
            "demo",
            "demo.web",
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].names[0], "orders-ro");
        assert_eq!(entries[1].addresses, vec!["10.42.2.200".to_string()]);
    }

    /// Copies of a web service *are* interchangeable, which is the whole
    /// point of having several. They share the one name.
    #[test]
    fn a_service_puts_every_copy_behind_one_name() {
        let entries = entries_for(
            &[("web".to_string(), None, None)],
            &addresses(&[("web", 1, "10.42.2.5"), ("web", 2, "10.42.2.6")]),
            "demo",
            "demo.web",
        );
        assert_eq!(entries.len(), 1, "no read pool for something with no roles");
        assert_eq!(entries[0].addresses.len(), 2);
    }

    /// The pool's name follows the primary's, whatever the operator chose.
    ///
    /// `-ro` in the first label rather than appended to the whole name:
    /// `db-ro.example.com` is a hostname and `db.example.com-ro` is not.
    /// It degenerates to what every database had before a name could be
    /// chosen, which is what keeps this from moving anything.
    #[test]
    fn the_read_pool_is_named_after_the_primary() {
        assert_eq!(pool_name("db.example.com"), "db-ro.example.com");
        assert_eq!(
            pool_name("orders.db-test.node.example"),
            "orders-ro.db-test.node.example"
        );
        assert_eq!(pool_name("db"), "db-ro", "a name can be one label");
    }

    /// Three ways to say it: bare inside the project, qualified by the
    /// project, and the full name the certificate will be for — so what
    /// an application connects to inside is what it would connect to
    /// from outside.
    #[test]
    fn a_name_is_written_short_and_long() {
        let entries = entries_for(
            // The qualified name, given rather than built: what a
            // database is called is the operator's choice now, and this
            // is the derivation it starts from.
            &[(
                "orders".to_string(),
                None,
                Some("orders.db-test.node.example".to_string()),
            )],
            &addresses(&[("orders", 1, "10.42.2.200")]),
            "db-test",
            "db-test.web",
        );
        let file = render(&entries);

        assert!(file.contains("10.42.2.200\torders orders.db-test orders.db-test.node.example"));
        assert!(file.contains("127.0.0.1\tlocalhost"));
    }

    /// The bug this fixes, measured on the Alpine node: a copy held for
    /// another node named itself under *that* node's domain, so the same
    /// database answered to a different qualified name depending on
    /// where you read it — and the certificate, which is the owner's,
    /// matched neither. `psql` said so:
    ///
    /// ```text
    /// server certificate for "orders" (and 5 other names) does not match
    /// host name "orders-ro.db-test.testing.example"
    /// ```
    ///
    /// The name belongs to the database, not to the machine holding a
    /// copy of it.
    #[test]
    fn a_copy_is_named_under_its_owners_domain_not_the_holders() {
        let entries = entries_for(
            &[
                (
                    "orders".to_string(),
                    Some(1),
                    Some("orders.db-test.owner.example".into()),
                ),
                (
                    "web".to_string(),
                    None,
                    Some("web.db-test.holder.example".into()),
                ),
            ],
            &addresses(&[("orders", 2, "10.42.2.254"), ("web", 1, "10.42.2.5")]),
            "db-test",
            "db-test.web",
        );
        let file = render(&entries);

        assert!(
            file.contains("orders-ro.db-test.owner.example"),
            "the copy answers to the owner's name: {file}"
        );
        assert!(
            !file.contains("orders-ro.db-test.holder.example"),
            "and not to this machine's: {file}"
        );
        assert!(
            file.contains("web.db-test.holder.example"),
            "a service of this node's own keeps its own domain: {file}"
        );
    }

    /// A node with no domain has nothing to qualify with, and the short
    /// names are all there is.
    #[test]
    fn without_a_domain_the_short_names_stand_alone() {
        let file = render(&entries_for(
            &[("orders".to_string(), None, None)],
            &addresses(&[("orders", 1, "10.42.2.200")]),
            "db-test",
            "db-test.web",
        ));
        assert!(
            file.contains("10.42.2.200\torders orders.db-test\n"),
            "{file}"
        );
    }

    /// Several copies are several lines, which is the form every
    /// resolver reads the same way.
    #[test]
    fn a_name_with_several_copies_is_several_lines() {
        let file = render(&[Entry {
            names: vec!["web".into(), "web.shared".into()],
            addresses: vec!["10.42.1.5".into(), "10.42.1.6".into()],
        }]);
        assert_eq!(file.matches("web web.shared").count(), 2, "{file}");
    }

    /// A pool written in one order is not a pool: every application
    /// container takes the first address that answers, so they would
    /// all read from one replica while the others held data nobody
    /// looked at.
    #[test]
    fn each_reader_is_given_the_pool_in_its_own_order() {
        let rows = [("orders".to_string(), Some(1), None)];
        let places = addresses(&[
            ("orders", 1, "10.42.2.200"),
            ("orders", 2, "10.42.2.251"),
            ("orders", 3, "10.42.2.252"),
            ("orders", 4, "10.42.2.253"),
        ]);

        let first_of = |reader: &str| {
            entries_for(&rows, &places, "demo", reader)
                .into_iter()
                .find(|entry| entry.names[0] == "orders-ro")
                .expect("a pool")
                .addresses
        };

        let orders: std::collections::BTreeSet<String> = ["demo.web", "demo.web.2", "demo.api"]
            .iter()
            .map(|reader| first_of(reader)[0].clone())
            .collect();
        assert!(
            orders.len() > 1,
            "every reader was sent to the same replica"
        );

        // Stable, though: rewriting the file must not move a client's
        // replica underneath it.
        assert_eq!(first_of("demo.web"), first_of("demo.web"));
        // And every reader is offered all of them, in some order — this
        // rotates, it does not drop.
        let mut sorted = first_of("demo.api");
        sorted.sort();
        assert_eq!(sorted, vec!["10.42.2.251", "10.42.2.252", "10.42.2.253"]);
    }

    /// localhost is there even when the project holds nothing, because a
    /// container whose `/etc/hosts` lost it is one where half of
    /// everything stops resolving.
    #[test]
    fn localhost_survives_an_empty_project() {
        let file = render(&[]);
        assert!(file.contains("127.0.0.1\tlocalhost"));
        assert!(file.contains("::1\tlocalhost"));
    }

    /// In place, never a rename: a bind mount follows the inode it was
    /// made from, so a renamed file would leave the container reading
    /// the old one for ever while the node edited a new one.
    #[test]
    fn rewriting_keeps_the_same_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = write(dir.path(), "demo.web", &[]).expect("written");
        let inode = std::fs::metadata(&first).expect("stat");

        let second = write(
            dir.path(),
            "demo.web",
            &[Entry {
                names: vec!["orders".into()],
                addresses: vec!["10.42.2.200".into()],
            }],
        )
        .expect("written again");

        assert_eq!(first, second);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                inode.ino(),
                std::fs::metadata(&second).expect("stat").ino(),
                "the mount would still be pointing at the old one"
            );
        }
        assert!(std::fs::read_to_string(&second)
            .expect("read")
            .contains("10.42.2.200"));
    }
}
