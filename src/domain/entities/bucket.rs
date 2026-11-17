//! A bucket: a namespace of objects in one store, with who may read it from
//! a browser and how long what it holds is kept.

use serde::Serialize;

use crate::domain::values::bucket_name::BucketName;
use crate::domain::values::cors_origin::CorsOrigin;

pub const DAYS: &str = "days are a whole number, at least one";

pub const NO_ROOM: &str = "a bucket needs a maximum size larger than nothing";

pub const ABORT_UPLOADS_DAYS: u32 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Bucket {
    pub id: String,
    pub service_id: String,
    pub name: BucketName,
    /// Bytes. Garage has quotas per bucket and none per store, so this is
    /// what keeps a full bucket from filling a disk other services share.
    pub max_size: u64,
    /// The name its objects are read at without a signature. Garage finds
    /// the bucket by it, so it is also one of the bucket's aliases there.
    pub public_hostname: Option<String>,
    pub cors_origins: Vec<CorsOrigin>,
    pub expire_days: Option<u32>,
    pub abort_uploads_days: Option<u32>,
}

impl Bucket {
    pub fn new(
        id: String,
        service_id: String,
        name: BucketName,
        max_size: u64,
    ) -> Result<Self, &'static str> {
        if max_size == 0 {
            return Err(NO_ROOM);
        }
        Ok(Self {
            id,
            service_id,
            name,
            max_size,
            public_hostname: None,
            cors_origins: Vec::new(),
            expire_days: None,
            abort_uploads_days: Some(ABORT_UPLOADS_DAYS),
        })
    }

    /// Zero days is refused rather than read as "never": S3 refuses a rule
    /// of zero days, and "never" is the field left empty.
    pub fn set_expiry(
        &mut self,
        expire_days: Option<u32>,
        abort_uploads_days: Option<u32>,
    ) -> Result<(), &'static str> {
        if expire_days == Some(0) || abort_uploads_days == Some(0) {
            return Err(DAYS);
        }
        self.expire_days = expire_days;
        self.abort_uploads_days = abort_uploads_days;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(max_size: u64) -> Result<Bucket, &'static str> {
        Bucket::new(
            "b1".into(),
            "s1".into(),
            BucketName::parse("uploads").expect("a name"),
            max_size,
        )
    }

    fn uploads() -> Bucket {
        named(1024 * 1024 * 1024).expect("a bucket")
    }

    #[test]
    fn a_bucket_with_no_room_is_refused() {
        assert_eq!(named(0), Err(NO_ROOM));
        assert_eq!(named(1).map(|bucket| bucket.max_size), Ok(1));
    }

    #[test]
    fn a_new_bucket_is_private_kept_for_ever_and_sweeps_abandoned_uploads() {
        let bucket = uploads();
        assert_eq!(bucket.public_hostname, None);
        assert!(bucket.cors_origins.is_empty());
        assert_eq!(bucket.expire_days, None);
        assert_eq!(bucket.abort_uploads_days, Some(ABORT_UPLOADS_DAYS));
    }

    #[test]
    fn zero_days_is_refused_and_an_empty_field_is_never() {
        let mut bucket = uploads();
        assert_eq!(bucket.set_expiry(Some(0), None), Err(DAYS));
        assert_eq!(bucket.set_expiry(None, Some(0)), Err(DAYS));
        assert_eq!(bucket.abort_uploads_days, Some(ABORT_UPLOADS_DAYS));
        assert_eq!(bucket.set_expiry(Some(30), None), Ok(()));
        assert_eq!(
            (bucket.expire_days, bucket.abort_uploads_days),
            (Some(30), None)
        );
    }
}
