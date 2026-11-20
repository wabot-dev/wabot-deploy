//! Reaching a service of the same project on another node, by its name.
//!
//! ## Two hops the node makes, and none the container sees
//!
//! A name with no copy on this node resolves to a **stand-in** in
//! `10.42.255.0/24` — a range no bridge uses, since a project's index is at
//! most 254. Nothing holds that address: the container routes it to its
//! gateway like anything off its own subnet, and one `REDIRECT` in
//! `infra::isolation` hands every such connection to [`names`] on this node.
//! The kernel keeps the address that was asked for (`SO_ORIGINAL_DST`), and
//! the container's own address says which project asked, so the pair names
//! one service.
//!
//! [`names`] then opens the **door** of the node running a copy — one port
//! on its overlay address — and knocks with the project, the service, the
//! copy's slot and the port. The door admits only the nodes that hold a copy
//! of that project, and dials the copy from the host. So the copy sees the
//! connection come from its own bridge's gateway, which every engine here
//! already admits as a neighbour; nothing about who may connect to a
//! database changes because the client is on another machine.
//!
//! ## Why not the overlay ports
//!
//! The edges, the standbys and the owner's forwards already reach copies
//! through ports CNI maps on the overlay address, and a connection through
//! one arrives as the far node's overlay address. Reusing them would have
//! meant every copy of a project that spans nodes taking one — at its next
//! deployment, which for a database is a restart — and every engine's list
//! of who may connect learning the overlay. The door asks for neither, and
//! it is what keeps a project's copies closed to a node that holds nothing
//! of it.

pub mod door;
pub mod names;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::infra::network::errand::{Copy, Directory, Listed};

/// The range stand-ins are taken from.
pub const STAND_INS: &str = "10.42.255.0/24";

/// Where [`names`] listens for what the `REDIRECT` hands it.
pub const NAMES_PORT: u16 = 31100;

/// Where [`door`] listens, on the overlay address.
pub const DOOR_PORT: u16 = 31101;

/// The longest knock a door reads before it gives up on the caller.
const KNOCK_LIMIT: usize = 1024;

/// What a door answers once it reached the copy, before anything else.
const OPENED: &[u8] = b"ok\n";

pub fn stand_in_address(host: u8) -> Ipv4Addr {
    Ipv4Addr::new(10, 42, 255, host)
}

pub fn stand_in_host(address: Ipv4Addr) -> Option<u8> {
    match address.octets() {
        [10, 42, 255, host] if host != 0 && host != 255 => Some(host),
        _ => None,
    }
}

/// Which copies a name is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// An ordinary service: any copy.
    Any,
    /// A database's name: its primary.
    Primary,
    /// `<name>-ro`: its standbys, or its primary when it has none.
    Pool,
}

/// A name this node has no copy behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub name: String,
    pub service: String,
    pub role: Role,
}

/// The names in `directory` that only another node can answer.
///
/// `me` is this node's overlay address and `here` every `(service, slot)`
/// running on this node. A name with a copy here keeps resolving to it, as
/// it always has; a stand-in is only for what would otherwise not resolve.
pub fn wanted(directory: &Directory, me: &str, here: &BTreeSet<(String, u32)>) -> Vec<Wanted> {
    let mut wanted = Vec::new();
    for listed in &directory.services {
        let local: BTreeSet<u32> = here
            .iter()
            .filter(|(service, _)| *service == listed.service)
            .map(|(_, slot)| *slot)
            .collect();
        let remote: Vec<u32> = listed
            .copies
            .iter()
            .filter(|copy| copy.node != me)
            .map(|copy| copy.slot)
            .collect();
        let want = |name: String, role| Wanted {
            name,
            service: listed.service.clone(),
            role,
        };
        match listed.primary_slot {
            None => {
                if local.is_empty() && !remote.is_empty() {
                    wanted.push(want(listed.service.clone(), Role::Any));
                }
            }
            Some(primary) => {
                let primary_elsewhere = !local.contains(&primary) && remote.contains(&primary);
                if primary_elsewhere {
                    wanted.push(want(listed.service.clone(), Role::Primary));
                }
                let standby_here = local.iter().any(|slot| *slot != primary);
                let standby_elsewhere = remote.iter().any(|slot| *slot != primary);
                let pool_elsewhere = standby_elsewhere || (primary_elsewhere && !standby_here);
                if !standby_here && pool_elsewhere {
                    wanted.push(want(
                        format!(
                            "{}{}",
                            listed.service,
                            crate::infra::deploy::hosts::READ_ONLY
                        ),
                        Role::Pool,
                    ));
                }
            }
        }
    }
    wanted
}

/// What a stand-in's name asks for, from the directory.
pub fn resolve<'a>(directory: &'a Directory, name: &str) -> Option<(&'a Listed, Role)> {
    directory.services.iter().find_map(|listed| {
        if listed.service == name {
            return Some((
                listed,
                match listed.primary_slot {
                    Some(_) => Role::Primary,
                    None => Role::Any,
                },
            ));
        }
        let pool = format!(
            "{}{}",
            listed.service,
            crate::infra::deploy::hosts::READ_ONLY
        );
        (listed.primary_slot.is_some() && pool == name).then_some((listed, Role::Pool))
    })
}

/// The copies elsewhere a connection for `role` may go to.
pub fn candidates(listed: &Listed, role: Role, me: &str) -> Vec<Copy> {
    let remote = listed.copies.iter().filter(|copy| copy.node != me);
    let primary = listed.primary_slot;
    let chosen: Vec<Copy> = match role {
        Role::Any => remote.cloned().collect(),
        Role::Primary => remote
            .filter(|copy| Some(copy.slot) == primary)
            .cloned()
            .collect(),
        Role::Pool => {
            let standbys: Vec<Copy> = remote
                .clone()
                .filter(|copy| Some(copy.slot) != primary)
                .cloned()
                .collect();
            match standbys.is_empty() {
                false => standbys,
                true => remote
                    .filter(|copy| Some(copy.slot) == primary)
                    .cloned()
                    .collect(),
            }
        }
    };
    chosen
}

/// What a node says at another's door: which copy, and which of its ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knock {
    pub project: String,
    pub service: String,
    pub slot: u32,
    pub port: u16,
}

impl Knock {
    pub(crate) fn line(&self) -> Vec<u8> {
        let mut line = serde_json::to_vec(self).expect("a knock is plain data");
        line.push(b'\n');
        line
    }
}

/// Read one line, a byte at a time, so nothing the caller sent after it is
/// taken out of the stream.
async fn read_line<R: tokio::io::AsyncRead + Unpin>(stream: &mut R) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut line = Vec::new();
    loop {
        let byte = stream.read_u8().await?;
        if byte == b'\n' {
            return Ok(line);
        }
        if line.len() == KNOCK_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "a knock longer than any project's name",
            ));
        }
        line.push(byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(node: &str, slot: u32) -> Copy {
        Copy {
            node: node.into(),
            slot,
        }
    }

    fn listed(service: &str, primary_slot: Option<u32>, copies: Vec<Copy>) -> Listed {
        Listed {
            service: service.into(),
            qualified: None,
            ports: vec![5432],
            primary_slot,
            copies,
        }
    }

    fn here(pairs: &[(&str, u32)]) -> BTreeSet<(String, u32)> {
        pairs
            .iter()
            .map(|(service, slot)| (service.to_string(), *slot))
            .collect()
    }

    const ME: &str = "10.42.0.2";
    const OWNER: &str = "10.42.0.1";

    #[test]
    fn a_stand_in_is_an_address_no_bridge_uses() {
        assert_eq!(stand_in_address(7), Ipv4Addr::new(10, 42, 255, 7));
        assert_eq!(stand_in_host(Ipv4Addr::new(10, 42, 255, 7)), Some(7));
        assert_eq!(stand_in_host(Ipv4Addr::new(10, 42, 254, 7)), None);
        assert_eq!(stand_in_host(Ipv4Addr::new(10, 42, 255, 255)), None);
    }

    /// The case Jorge found: a service on one node, its database on the
    /// other, and no name for it.
    #[test]
    fn a_database_whose_primary_is_elsewhere_gets_a_stand_in_for_both_names() {
        let directory = Directory {
            services: vec![listed("orders", Some(1), vec![copy(OWNER, 1)])],
            ..Directory::default()
        };

        let wanted = wanted(&directory, ME, &here(&[("web", 1)]));

        assert_eq!(
            wanted,
            vec![
                Wanted {
                    name: "orders".into(),
                    service: "orders".into(),
                    role: Role::Primary
                },
                Wanted {
                    name: "orders-ro".into(),
                    service: "orders".into(),
                    role: Role::Pool
                },
            ]
        );
    }

    /// What has a copy here resolves to it, as it always did.
    #[test]
    fn a_name_with_a_copy_here_is_not_stood_in_for() {
        let directory = Directory {
            services: vec![
                listed("web", None, vec![copy(OWNER, 1), copy(ME, 2)]),
                listed("orders", Some(1), vec![copy(OWNER, 1), copy(ME, 2)]),
            ],
            ..Directory::default()
        };

        let wanted = wanted(&directory, ME, &here(&[("web", 2), ("orders", 2)]));

        assert_eq!(
            wanted,
            vec![Wanted {
                name: "orders".into(),
                service: "orders".into(),
                role: Role::Primary
            }],
            "the standby here is the pool, and only the primary is elsewhere"
        );
    }

    #[test]
    fn the_pool_goes_to_the_standbys_and_falls_back_to_the_primary() {
        let with = listed(
            "orders",
            Some(1),
            vec![copy(OWNER, 1), copy(OWNER, 2), copy("10.42.0.3", 3)],
        );
        let slots = |copies: Vec<Copy>| copies.iter().map(|c| c.slot).collect::<Vec<_>>();
        assert_eq!(slots(candidates(&with, Role::Pool, ME)), vec![2, 3]);
        assert_eq!(slots(candidates(&with, Role::Primary, ME)), vec![1]);

        let without = listed("orders", Some(1), vec![copy(OWNER, 1)]);
        assert_eq!(slots(candidates(&without, Role::Pool, ME)), vec![1]);
    }

    #[test]
    fn a_stand_ins_name_resolves_to_its_service_and_role() {
        let directory = Directory {
            services: vec![
                listed("web", None, vec![copy(OWNER, 1)]),
                listed("orders", Some(1), vec![copy(OWNER, 1)]),
            ],
            ..Directory::default()
        };
        let role = |name| resolve(&directory, name).map(|(_, role)| role);

        assert_eq!(role("web"), Some(Role::Any));
        assert_eq!(role("orders"), Some(Role::Primary));
        assert_eq!(role("orders-ro"), Some(Role::Pool));
        assert_eq!(role("web-ro"), None, "a service has no pool");
        assert_eq!(role("nothing"), None);
    }

    #[tokio::test]
    async fn a_knock_is_read_up_to_its_line_and_no_further() {
        let knock = Knock {
            project: "Shop".into(),
            service: "orders".into(),
            slot: 1,
            port: 5432,
        };
        let mut sent = knock.line();
        sent.extend_from_slice(b"what the client says first");
        let mut stream = std::io::Cursor::new(sent);

        let line = read_line(&mut stream).await.expect("a line");
        assert_eq!(serde_json::from_slice::<Knock>(&line).expect("json"), knock);
        let mut rest = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut rest)
            .await
            .expect("rest");
        assert_eq!(rest, b"what the client says first");

        let mut endless = std::io::Cursor::new(vec![b'x'; KNOCK_LIMIT + 10]);
        assert!(read_line(&mut endless).await.is_err());
    }
}
