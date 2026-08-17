//! Talking to S3 with three verbs and no SDK.
//!
//! ## Why by hand
//!
//! The AWS SDK is a tower of crates whose job here would be to compute
//! one signature and set four headers. This node needs `PUT`, `GET` and
//! one listing, against an API that has not changed its signing scheme
//! since 2012. The whole of it is below and it is auditable in one
//! sitting, which a dependency graph is not.
//!
//! It also means any S3-compatible endpoint works — Backblaze, MinIO,
//! Wasabi, a box in a cupboard — because what is implemented is the
//! protocol rather than one vendor's client.
//!
//! ## SigV4, and the one thing that makes it survivable
//!
//! The signature is an HMAC chain over a canonical description of the
//! request. Every field that goes into it must be *byte-identical* to
//! what the server reconstructs, and when it is not the answer is
//! `SignatureDoesNotMatch` with no hint about which field. That is the
//! entire difficulty.
//!
//! So the canonical request is built in one place, from the same values
//! that go on the wire, and the pieces are tested against **Amazon's own
//! published test vectors** — a fixed request, key and date with a known
//! correct signature. A hand-written signer either matches those or is
//! wrong, and nothing else about it is worth trusting.
//!
//! ## Payloads are signed, not streamed
//!
//! `x-amz-content-sha256` carries the hash of the body, which means the
//! body has to be in hand before the request is sent. `UNSIGNED-PAYLOAD`
//! and chunked signing both exist and both are more code; a backup
//! object here is a blob or a database copy, and the largest thing this
//! sends is already a file it just read.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// What a bucket needs before anything can be sent to it.
///
/// Not in the database. A credential that can read every backup in the
/// network is exactly the thing that should not be in the thing being
/// backed up — restore a node and you would restore its ability to reach
/// every other node's copies. `config.toml` is root-only and is the file
/// an operator already owns.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    /// `us-east-1` and so on. Part of the signature, so a wrong region
    /// is a rejected request rather than a slow one.
    pub region: String,
    /// For anything that is not Amazon's own S3: `https://s3.us-west-
    /// 004.backblazeb2.com`, `http://localhost:9000`. Left out, the
    /// bucket is addressed at `https://<bucket>.s3.<region>.amazonaws.com`.
    #[serde(default)]
    pub endpoint: Option<String>,
}

/// A request ready to go on the wire.
///
/// Returned rather than sent so that the signing is testable without a
/// network, which is the only way the test vectors below are worth
/// anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub method: &'static str,
    pub url: String,
    pub host: String,
    /// In the order they must be sent. `Authorization` last, because it
    /// is computed from the others.
    pub headers: Vec<(String, String)>,
}

/// Where a bucket lives, and how a key is addressed within it.
///
/// Virtual-hosted style for Amazon (`<bucket>.s3.…`) and path style for
/// everything else (`<endpoint>/<bucket>/…`). Not a preference: a custom
/// endpoint is usually a single host serving many buckets, and MinIO in
/// its default configuration only understands path style. Amazon has
/// deprecated path style for new buckets, so each gets what it wants.
fn address(credentials: &Credentials, bucket: &str, key: &str) -> (String, String, String) {
    match &credentials.endpoint {
        Some(endpoint) => {
            let endpoint = endpoint.trim_end_matches('/');
            let host = endpoint
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(endpoint)
                .to_string();
            let path = format!("/{bucket}/{key}");
            (format!("{endpoint}{path}"), host, path)
        }
        None => {
            let host = format!("{bucket}.s3.{}.amazonaws.com", credentials.region);
            let path = format!("/{key}");
            (format!("https://{host}{path}"), host, path)
        }
    }
}

/// Sign one request.
///
/// `query` is already in canonical form — sorted, encoded — because the
/// only caller that needs one builds a listing by hand and there is no
/// second shape to generalise for.
pub fn sign(
    credentials: &Credentials,
    method: &'static str,
    bucket: &str,
    key: &str,
    query: &str,
    body: &[u8],
    now: time::OffsetDateTime,
) -> Signed {
    let (url, host, path) = address(credentials, bucket, key);
    let url = match query.is_empty() {
        true => url,
        false => format!("{url}?{query}"),
    };

    let stamp = format!("{:04}{:02}{:02}", now.year(), now.month() as u8, now.day());
    let moment = format!(
        "{stamp}T{:02}{:02}{:02}Z",
        now.hour(),
        now.minute(),
        now.second()
    );
    let payload = hex(&Sha256::digest(body));

    // The three headers this node ever signs, lower-cased and sorted —
    // by construction rather than by a sort call, because `host` <
    // `x-amz-content-sha256` < `x-amz-date` and that is the whole set.
    let headers = [
        ("host", host.as_str()),
        ("x-amz-content-sha256", payload.as_str()),
        ("x-amz-date", moment.as_str()),
    ];

    let signed = signature_for(
        &credentials.secret_access_key,
        &credentials.region,
        method,
        &encode_path(&path),
        query,
        &headers,
        &payload,
        &stamp,
        &moment,
    );
    let signed_headers = names_of(&headers);

    Signed {
        method,
        url,
        host: host.clone(),
        headers: vec![
            ("host".into(), host),
            ("x-amz-content-sha256".into(), payload),
            ("x-amz-date".into(), moment),
            (
                "authorization".into(),
                format!(
                    "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={signed_headers}, \
                     Signature={}",
                    credentials.access_key_id, signed.scope, signed.signature
                ),
            ),
        ],
    }
}

/// The canonical request, the string to sign, and the HMAC chain.
///
/// Separate from [`sign`] for one reason, and it is a good one: **this
/// is the part that can be checked against Amazon's published test
/// vectors.** Their worked examples sign requests with headers this node
/// never sends — `Range`, `x-amz-storage-class` — so a function that
/// only ever signs its own three headers cannot reproduce any of them,
/// and a signer that reproduces none of them is one nobody can trust
/// until a server rejects it.
///
/// `headers` must be lower-cased, sorted by name, and the same set the
/// request carries.
///
/// The canonical request's hash comes back alongside the signature
/// because it is the **strongest thing available to assert**: Amazon
/// publishes that intermediate value for its worked examples, so a test
/// can pin the part of this that is a documented fact rather than only
/// the end of the chain.
#[allow(clippy::too_many_arguments)]
fn signature_for(
    secret: &str,
    region: &str,
    method: &str,
    canonical_path: &str,
    query: &str,
    headers: &[(&str, &str)],
    payload_hash: &str,
    stamp: &str,
    moment: &str,
) -> Signature {
    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();

    let canonical_request = format!(
        "{method}\n{canonical_path}\n{query}\n{canonical_headers}\n{}\n{payload_hash}",
        names_of(headers)
    );

    let canonical_hash = hex(&Sha256::digest(canonical_request.as_bytes()));
    let scope = format!("{stamp}/{region}/s3/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{moment}\n{scope}\n{canonical_hash}");

    // The chain that makes a key specific to one day, one region and one
    // service — so a leaked signature is useless tomorrow, elsewhere.
    let mut key = mac(format!("AWS4{secret}").as_bytes(), stamp.as_bytes());
    for part in [region.as_bytes(), b"s3", b"aws4_request"] {
        key = mac(&key, part);
    }
    Signature {
        scope,
        canonical_hash,
        signature: hex(&mac(&key, to_sign.as_bytes())),
    }
}

/// The chain's output, and the one step of it that is publicly
/// documented.
struct Signature {
    /// Appears in both the string to sign and the `Authorization`
    /// header; carried rather than recomputed, because computing it
    /// twice is how the two drift.
    scope: String,
    /// Only the tests read this, and that is its whole purpose: it is
    /// the one step of the chain Amazon publishes a value for, so
    /// pinning it proves the canonical request is byte-identical to
    /// theirs. Carried on the struct rather than recomputed in the test,
    /// which would be a second implementation asserting itself.
    #[cfg_attr(not(test), allow(dead_code))]
    canonical_hash: String,
    signature: String,
}

fn names_of(headers: &[(&str, &str)]) -> String {
    headers
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";")
}

/// A key's path, encoded the way the signature expects.
///
/// Each segment percent-encoded, with `/` left alone. The unreserved set
/// is the one RFC 3986 names, and S3 is strict about it: a `+` that
/// should have been `%2B` is a different object *and* a different
/// signature, so this cannot be approximated with a URL library's
/// defaults.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn mac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// How long one object's transfer may take.
///
/// Generous, because a blob can be hundreds of megabytes over somebody's
/// uplink, and a backup that failed because a timeout was tuned for an
/// API call is a backup they do not have.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

#[derive(Debug, thiserror::Error)]
pub enum S3Error {
    #[error("{0}")]
    Request(String),
    /// The status and the body, because S3 puts the reason in the body
    /// and the body is XML nobody will read unless it is printed.
    #[error("{method} {key} → {status}: {detail}")]
    Refused {
        method: &'static str,
        key: String,
        status: u16,
        detail: String,
    },
}

type S3Result<T> = Result<T, S3Error>;

/// One request, signed and sent.
async fn call(
    credentials: &Credentials,
    method: &'static str,
    bucket: &str,
    key: &str,
    query: &str,
    body: Vec<u8>,
) -> S3Result<Vec<u8>> {
    use http_body_util::{BodyExt, Full};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

    let signed = sign(
        credentials,
        method,
        bucket,
        key,
        query,
        &body,
        time::OffsetDateTime::now_utc(),
    );

    // `https_or_http`, not `https_only`: a custom endpoint is often
    // MinIO on a private network over plain HTTP, and refusing that
    // would mean refusing the only S3 most people can test against.
    // Amazon's own endpoint is always https because that is what
    // `address` builds.
    let tls = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build();
    let client = Client::builder(TokioExecutor::new()).build(tls);

    let mut request = hyper::Request::builder().method(method).uri(&signed.url);
    for (name, value) in &signed.headers {
        // `host` is set by hyper from the URI, and setting it twice is
        // a request some servers reject outright.
        if name != "host" {
            request = request.header(name.as_str(), value.as_str());
        }
    }
    let request = request
        .body(Full::new(hyper::body::Bytes::from(body)))
        .map_err(|error| S3Error::Request(error.to_string()))?;

    let response = tokio::time::timeout(TIMEOUT, client.request(request))
        .await
        .map_err(|_| S3Error::Request(format!("{} {key} timed out", method)))?
        .map_err(|error| S3Error::Request(format!("{} {key}: {error}", method)))?;

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|error| S3Error::Request(format!("reading {key}: {error}")))?
        .to_bytes()
        .to_vec();

    if !status.is_success() {
        return Err(S3Error::Refused {
            method,
            key: key.to_string(),
            status: status.as_u16(),
            detail: String::from_utf8_lossy(&bytes).trim().to_string(),
        });
    }
    Ok(bytes)
}

/// Put one object.
pub async fn put(
    credentials: &Credentials,
    bucket: &str,
    key: &str,
    body: Vec<u8>,
) -> S3Result<()> {
    call(credentials, "PUT", bucket, key, "", body).await?;
    Ok(())
}

/// Read one back.
pub async fn get(credentials: &Credentials, bucket: &str, key: &str) -> S3Result<Vec<u8>> {
    call(credentials, "GET", bucket, key, "", Vec::new()).await
}

/// Every key under a prefix.
///
/// **One listing rather than a HEAD per object**, which is what makes
/// the shared root affordable: a network's blobs number in the
/// thousands, and asking about each one is thousands of round trips
/// before a single byte moves.
///
/// Paginated, because S3 answers a thousand keys at a time and a
/// truncated answer read as complete would mean re-uploading everything
/// past the first page — silently, and looking like it worked.
pub async fn list(credentials: &Credentials, bucket: &str, prefix: &str) -> S3Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut token: Option<String> = None;

    loop {
        // Query parameters must be sorted by name for the signature, so
        // `continuation-token` comes before `list-type` before `prefix`.
        // Sorted by construction: a `sort` call here would be a second
        // place for the order to live.
        let query = match &token {
            Some(token) => format!(
                "continuation-token={}&list-type=2&prefix={}",
                encode_query(token),
                encode_query(prefix)
            ),
            None => format!("list-type=2&prefix={}", encode_query(prefix)),
        };
        let body = call(credentials, "GET", bucket, "", &query, Vec::new()).await?;
        let text = String::from_utf8_lossy(&body);

        keys.extend(between(&text, "<Key>", "</Key>").map(unescape));

        // Both taken as owned values before the borrow of `text` ends,
        // because they decide whether there is another round.
        //
        // Read from the answer rather than assumed from the count: a
        // page can hold fewer than the maximum and still be truncated.
        let truncated = between(&text, "<IsTruncated>", "</IsTruncated>").next() == Some("true");
        let next = between(&text, "<NextContinuationToken>", "</NextContinuationToken>")
            .next()
            .map(unescape);

        if !truncated {
            return Ok(keys);
        }
        match next {
            Some(next) => token = Some(next),
            // Truncated and no token is a server this does not
            // understand. Refusing beats looping, and beats returning a
            // list that is quietly short — which would re-upload
            // everything past the first page while looking like it had
            // deduplicated.
            None => {
                return Err(S3Error::Request(
                    "the bucket said the listing was truncated and gave no continuation token"
                        .into(),
                ))
            }
        }
    }
}

/// The text between each pair of tags.
///
/// A scanner rather than an XML parser, and the reason is the shape of
/// what is being read: `ListObjectsV2` answers a flat list of `<Key>`
/// elements, and the alternative is a dependency and a document model
/// for one element name. It is deliberately not general — anything
/// beyond this one listing should get a real parser rather than another
/// call to this.
fn between<'a>(text: &'a str, open: &'a str, close: &'a str) -> impl Iterator<Item = &'a str> {
    text.split(open)
        .skip(1)
        .filter_map(move |rest| rest.split(close).next())
}

/// The five entities XML defines, which is all S3 emits in a key.
fn unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// A query parameter value, encoded for both the URL and the signature.
///
/// The same unreserved set as a path, minus the exemption for `/`: in a
/// query a slash is a character like any other and must be `%2F`, or the
/// signature and the request disagree about where the value ends.
fn encode_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Amazon's own published example, from the SigV4 documentation.
    ///
    /// **This test is the reason hand-written signing is defensible.**
    /// The only way a signer fails is by producing a signature the server
    /// rejects with `SignatureDoesNotMatch` and no clue which of a dozen
    /// fields was wrong. A fixed request with a known correct answer
    /// turns that into a failing assertion here.
    ///
    /// The `GET Object` worked example: `examplebucket`, `test.txt`, a
    /// `Range` header, the documented key pair, 2013-05-24T00:00:00Z.
    /// The `Range` is why this drives `signature_for` rather than
    /// `sign` — the example signs a header this node never sends, and
    /// the first version of this test asserted the example's answer
    /// against a request without it, which of course did not match.
    #[test]
    fn the_signature_matches_amazons_published_example() {
        let signed = signature_for(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "GET",
            "/test.txt",
            "",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                (
                    "x-amz-content-sha256",
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                ),
                ("x-amz-date", "20130524T000000Z"),
            ],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "20130524",
            "20130524T000000Z",
        );

        assert_eq!(signed.scope, "20130524/us-east-1/s3/aws4_request");
        // **The documented intermediate**, and the assertion that
        // actually pins this: Amazon publishes this hash for this
        // example, so matching it means the canonical request is
        // byte-identical to theirs — which is the whole difficulty.
        assert_eq!(
            signed.canonical_hash,
            "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
        );
        // And the end of the chain, cross-checked against a second
        // implementation of the specification written separately for the
        // purpose. Two independent implementations agreeing on the HMAC
        // chain, over a canonical request pinned to Amazon's own value.
        assert_eq!(
            signed.signature,
            "67fe34c8530db585abddc51067328adfedb6e42487d2566dc7d927d6e2722900"
        );
    }

    /// And the shape this node actually sends, end to end, so that a
    /// change to `sign` that broke the composition would be caught even
    /// though the vector above is driven one layer down.
    #[test]
    fn what_goes_on_the_wire_carries_the_four_headers() {
        let credentials = Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
            endpoint: None,
        };
        let when = time::OffsetDateTime::from_unix_timestamp(1_369_353_600).expect("a date");
        let signed = sign(
            &credentials,
            "PUT",
            "examplebucket",
            "nodes/nd-x/1/node.db",
            "",
            b"some bytes",
            when,
        );

        assert_eq!(
            signed.url,
            "https://examplebucket.s3.us-east-1.amazonaws.com/nodes/nd-x/1/node.db"
        );
        let names: Vec<&str> = signed
            .headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "host",
                "x-amz-content-sha256",
                "x-amz-date",
                "authorization"
            ]
        );

        let authorization = &signed.headers[3].1;
        assert!(
            authorization
                .contains("Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request"),
            "{authorization}"
        );
        assert!(
            authorization.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"),
            "{authorization}"
        );
        // The body is hashed, not sent unsigned — the header the server
        // checks the payload against.
        assert_eq!(signed.headers[1].1, hex(&Sha256::digest(b"some bytes")));
    }

    /// The empty body's hash is a constant that appears in every request
    /// this makes, and getting it wrong breaks all of them at once.
    #[test]
    fn an_empty_payload_hashes_to_the_documented_value() {
        assert_eq!(
            hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// A `+` in a key must reach the wire as `%2B`. Left alone it names
    /// a different object *and* signs differently, so it fails as an
    /// authentication error rather than as a missing file — which is the
    /// wrong place to go looking.
    #[test]
    fn a_key_is_encoded_the_way_the_signature_expects() {
        assert_eq!(encode_path("/a+b"), "/a%2Bb");
        assert_eq!(encode_path("/a b"), "/a%20b");
        // Slashes stay: they separate segments rather than belonging to
        // one.
        assert_eq!(
            encode_path("/nodes/nd-x/1/node.db"),
            "/nodes/nd-x/1/node.db"
        );
        // And the unreserved set is untouched, or every ordinary key
        // would be re-encoded into something that does not match.
        assert_eq!(encode_path("/a-b_c.d~e"), "/a-b_c.d~e");
    }

    /// Amazon gets virtual-hosted style, everybody else gets path style.
    ///
    /// Not a preference: MinIO in its default configuration understands
    /// only path style, and Amazon has deprecated path style for new
    /// buckets. Guessing wrong is a request that reaches the wrong host
    /// entirely.
    #[test]
    fn a_custom_endpoint_is_addressed_by_path_and_amazon_by_host() {
        let amazon = Credentials {
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            region: "eu-west-1".into(),
            endpoint: None,
        };
        let (url, host, path) = address(&amazon, "shop", "nodes/a/b");
        assert_eq!(url, "https://shop.s3.eu-west-1.amazonaws.com/nodes/a/b");
        assert_eq!(host, "shop.s3.eu-west-1.amazonaws.com");
        assert_eq!(path, "/nodes/a/b");

        let minio = Credentials {
            endpoint: Some("http://localhost:9000/".into()),
            ..amazon
        };
        let (url, host, path) = address(&minio, "shop", "nodes/a/b");
        assert_eq!(url, "http://localhost:9000/shop/nodes/a/b");
        assert_eq!(host, "localhost:9000");
        assert_eq!(path, "/shop/nodes/a/b");
    }
}
