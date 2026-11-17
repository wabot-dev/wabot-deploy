//! Where a request to a store's name goes, read off the request alone.
//!
//! Garage answers the S3 API and public reads on two ports, and a bucket
//! has one name — as on Amazon, where the same URL takes a signed upload
//! and an anonymous read. So the edge reads whether it was signed.

/// Amazon's own answer to a signature it no longer takes, word for word:
/// it is what a search for the error finds.
pub const V2_REFUSED: &str = "The authorization mechanism you have provided is not supported. \
                              Please use AWS4-HMAC-SHA256.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signature {
    None,
    V2,
    V4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Api,
    Public,
    /// Signature version 2. Garage reads it as no signature at all and
    /// answers that it has no anonymous access, which sends somebody
    /// looking at their key rather than at how the URL was signed.
    RefuseV2,
}

pub fn signature(authorization: Option<&str>, query: Option<&str>) -> Signature {
    if let Some(authorization) = authorization {
        return match authorization.trim_start().starts_with("AWS ") {
            true => Signature::V2,
            false => Signature::V4,
        };
    }
    let names = query
        .unwrap_or_default()
        .split('&')
        .map(|pair| pair.split_once('=').map_or(pair, |(name, _)| name));
    let mut signature = Signature::None;
    for name in names {
        if name.eq_ignore_ascii_case("X-Amz-Signature")
            || name.eq_ignore_ascii_case("X-Amz-Credential")
        {
            return Signature::V4;
        }
        if name == "AWSAccessKeyId" {
            signature = Signature::V2;
        }
    }
    signature
}

/// An unsigned write, or any unsigned request to a private bucket, goes to
/// the API, which refuses it as Amazon would. A CORS preflight is unsigned
/// by definition and answered there too: the rules are the API's.
pub fn destination(method: &str, signature: Signature, public: bool) -> Destination {
    match signature {
        Signature::V2 => Destination::RefuseV2,
        Signature::V4 => Destination::Api,
        Signature::None if public && matches!(method, "GET" | "HEAD") => Destination::Public,
        Signature::None => Destination::Api,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_signed_in_a_header_or_in_its_url_is_signed() {
        assert_eq!(
            signature(
                Some("AWS4-HMAC-SHA256 Credential=GK1/20261117/wabot/s3/aws4_request, …"),
                None
            ),
            Signature::V4
        );
        assert_eq!(
            signature(
                None,
                Some(
                    "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=GK1%2F…&X-Amz-Signature=ab"
                )
            ),
            Signature::V4
        );
        assert_eq!(signature(Some("AWS GK1:c2lnbmF0dXJl"), None), Signature::V2);
        assert_eq!(
            signature(
                None,
                Some("AWSAccessKeyId=GK1&Signature=c2ln&Expires=1763400000")
            ),
            Signature::V2
        );
        assert_eq!(
            signature(None, Some("versionId=3&list-type=2")),
            Signature::None
        );
        assert_eq!(signature(None, None), Signature::None);
    }

    /// What boto3 does when it is told nothing in `us-east-1`, measured
    /// against Garage: the URL works on Amazon and failed here with a
    /// sentence about anonymous access.
    #[test]
    fn signature_version_2_is_refused_with_amazons_own_sentence_whatever_the_bucket() {
        for public in [true, false] {
            assert_eq!(
                destination("GET", Signature::V2, public),
                Destination::RefuseV2
            );
        }
    }

    #[test]
    fn only_an_unsigned_read_of_a_public_bucket_is_a_public_read() {
        assert_eq!(
            destination("GET", Signature::None, true),
            Destination::Public
        );
        assert_eq!(
            destination("HEAD", Signature::None, true),
            Destination::Public
        );
        assert_eq!(destination("GET", Signature::V4, true), Destination::Api);
        assert_eq!(destination("GET", Signature::None, false), Destination::Api);
        assert_eq!(destination("PUT", Signature::None, true), Destination::Api);
        assert_eq!(
            destination("OPTIONS", Signature::None, true),
            Destination::Api
        );
    }
}
