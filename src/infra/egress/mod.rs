//! The node as the resolver of every project whose list is not anywhere
//! (`docs/egress.md`, "Names").
//!
//! A name the list holds is asked upstream, and the addresses in the answer
//! are opened in the kernel before the answer is handed back, so the
//! program's first connection finds them open. Any other name is answered
//! "no such name" without being asked, because the question is what leaks
//! it.

use std::net::Ipv4Addr;
use std::time::Duration;

use hickory_server::proto::op::{Message, MessageType, ResponseCode};
use hickory_server::proto::rr::RData;

use crate::domain::values::destinations::Destinations;

pub mod exit;
pub mod resolver;

pub const EXIT_PORT: u16 = 31102;
pub const RESOLVER_PORT: u16 = 31103;

/// How long an address stays open at the least, whatever the answer's TTL:
/// a program that caches past it, or keeps a connection pool, would
/// otherwise find a name it was allowed failing for no reason it can see.
pub const OPEN_AT_LEAST: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Refuse,
    /// Ask upstream, and open what comes back on these ports.
    Ask {
        opens: Vec<Option<u16>>,
    },
}

pub fn decide(destinations: &Destinations, name: &str) -> Decision {
    if destinations.is_anywhere() {
        return Decision::Ask { opens: Vec::new() };
    }
    let opens = destinations.ports_for(name);
    match opens.is_empty() && !destinations.asks_about_any_name() {
        true => Decision::Refuse,
        false => Decision::Ask { opens },
    }
}

/// The IPv4 addresses an answer gives, and how long the one cached longest
/// may be cached for.
pub fn addresses_in(answer: &Message) -> (Vec<Ipv4Addr>, Duration) {
    let mut addresses = Vec::new();
    let mut longest = 0;
    for record in &answer.answers {
        if let RData::A(address) = &record.data {
            addresses.push(address.0);
            longest = longest.max(record.ttl);
        }
    }
    addresses.sort_unstable();
    addresses.dedup();
    (addresses, Duration::from_secs(u64::from(longest)))
}

/// An answer to `query` that carries no records, only `code`.
pub fn answer_with(query: &Message, code: ResponseCode) -> Message {
    let mut answer = Message::error_msg(query.metadata.id, query.metadata.op_code, code);
    answer.metadata.message_type = MessageType::Response;
    answer.metadata.recursion_desired = query.metadata.recursion_desired;
    answer.metadata.recursion_available = true;
    answer.queries = query.queries.clone();
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_server::proto::op::{OpCode, Query};
    use hickory_server::proto::rr::rdata::{A, CNAME};
    use hickory_server::proto::rr::{Name, Record, RecordType};

    fn list(text: &str) -> Destinations {
        Destinations::parse(text).expect("a list")
    }

    #[test]
    fn a_name_on_the_list_is_asked_and_opened_on_its_lines_ports() {
        assert_eq!(
            decide(&list("api.example.com:443"), "api.example.com"),
            Decision::Ask {
                opens: vec![Some(443)]
            }
        );
    }

    #[test]
    fn a_name_off_a_restrictive_list_is_refused_without_being_asked() {
        assert_eq!(
            decide(
                &list("10.0.0.0/8\napi.example.com"),
                "leak.attacker.example"
            ),
            Decision::Refuse
        );
        assert_eq!(decide(&list(""), "api.example.com"), Decision::Refuse);
    }

    #[test]
    fn a_project_that_reaches_every_address_on_a_port_resolves_any_name() {
        assert_eq!(
            decide(&list("0.0.0.0/0:443"), "anything.example.com"),
            Decision::Ask { opens: Vec::new() }
        );
        assert_eq!(
            decide(&list("0.0.0.0/0"), "anything.example.com"),
            Decision::Ask { opens: Vec::new() }
        );
    }

    #[test]
    fn every_address_behind_a_cname_is_opened_for_as_long_as_the_longest_is_cached() {
        let name = |text: &str| Name::from_ascii(text).expect("a name");
        let mut answer = Message::response(7, OpCode::Query);
        answer.answers = vec![
            Record::from_rdata(
                name("api.example.com."),
                300,
                RData::CNAME(CNAME(name("edge.cdn.example."))),
            ),
            Record::from_rdata(
                name("edge.cdn.example."),
                60,
                RData::A(A("198.51.100.7".parse().expect("an address"))),
            ),
            Record::from_rdata(
                name("edge.cdn.example."),
                120,
                RData::A(A("192.0.2.9".parse().expect("an address"))),
            ),
        ];

        let (addresses, longest) = addresses_in(&answer);
        assert_eq!(
            addresses,
            vec![
                "192.0.2.9".parse::<Ipv4Addr>().expect("an address"),
                "198.51.100.7".parse().expect("an address"),
            ]
        );
        assert_eq!(longest, Duration::from_secs(120));
    }

    #[test]
    fn a_refusal_answers_the_question_that_was_asked() {
        let mut query = Message::query();
        query.metadata.recursion_desired = true;
        query.queries = vec![Query::query(
            Name::from_ascii("leak.attacker.example.").expect("a name"),
            RecordType::A,
        )];

        let refused = answer_with(&query, ResponseCode::NXDomain);
        assert_eq!(refused.metadata.id, query.metadata.id);
        assert_eq!(refused.metadata.message_type, MessageType::Response);
        assert_eq!(refused.metadata.response_code, ResponseCode::NXDomain);
        assert_eq!(refused.queries, query.queries);
        assert!(refused.answers.is_empty());
    }
}
