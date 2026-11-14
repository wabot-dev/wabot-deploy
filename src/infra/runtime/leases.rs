use std::collections::HashMap;
use std::time::Duration;

use containerd_client::services::v1::leases_client::LeasesClient;
use containerd_client::services::v1::CreateRequest;
use time::OffsetDateTime;

use super::client::{ClientError, ClientResult, Containerd};

const EXPIRES: &str = "containerd.io/gc.expire";

/// Keep what is written under `id` from the collector until `held_for` has passed.
///
/// An expiring lease rather than a `gc.root` label: a root would keep an
/// abandoned push for ever, and the collector deletes an expired lease itself.
pub async fn hold(client: &Containerd, id: &str, held_for: Duration) -> ClientResult<()> {
    let result = LeasesClient::new(client.channel())
        .create(client.request(CreateRequest {
            id: id.to_string(),
            labels: expiring(OffsetDateTime::now_utc() + held_for),
        }))
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(status) if status.code() == tonic::Code::AlreadyExists => Ok(()),
        Err(source) => Err(ClientError::Call {
            call: "Leases.Create",
            source,
        }),
    }
}

fn expiring(at: OffsetDateTime) -> HashMap<String, String> {
    let at = at.to_offset(time::UtcOffset::UTC);
    HashMap::from([(
        EXPIRES.to_string(),
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            at.year(),
            at.month() as u8,
            at.day(),
            at.hour(),
            at.minute(),
            at.second()
        ),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_expiry_is_written_as_the_time_containerd_parses() {
        let at = OffsetDateTime::from_unix_timestamp(1_794_650_000).unwrap();
        let labels = expiring(at);

        assert_eq!(labels.len(), 1);
        assert_eq!(labels[EXPIRES], "2026-11-14T09:53:20Z");
    }

    #[test]
    fn the_expiry_is_in_utc_whatever_the_offset_handed_in() {
        let utc = OffsetDateTime::from_unix_timestamp(1_794_650_000).unwrap();
        let bogota = utc.to_offset(time::UtcOffset::from_hms(-5, 0, 0).unwrap());

        assert_eq!(expiring(bogota), expiring(utc));
    }
}
