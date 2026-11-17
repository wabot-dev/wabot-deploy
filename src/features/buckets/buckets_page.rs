//! A store's buckets, how to reach them, and the form that makes one.

use hypertext::prelude::*;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::domain::policies::{garage, presets};
use crate::features::buckets::texts::t;
use crate::features::services::service_tabs_component::{service_tabs, ServicePlace};
use crate::infra::garage::Usage;
use crate::infra::node::memory::human;
use crate::shell::layouts::layout;

pub struct Buckets<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub may_deploy: bool,
    pub error: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    /// With what the running store says each one holds, when it answered.
    pub buckets: &'a [(Bucket, Option<Usage>)],
    pub reserved: u64,
    pub disk: Option<u64>,
    pub running: bool,
}

pub fn render<'a>(page: &'a Buckets<'a>) -> impl Renderable + 'a {
    let base = format!(
        "/projects/{}/services/{}/buckets",
        page.project.slug, page.service.slug
    );
    let left = page.disk.map(|disk| disk.saturating_sub(page.reserved));
    rsx! {
    (layout::style_tag())
        (service_tabs(&page.project.slug, page.service, page.may_deploy, ServicePlace::Buckets))
        @if let Some(message) = &page.error {
            (layout::error_note(message))
        }

        <section class="card stack" id="connect">
            <p class="card-label">(t("How to reach it"))</p>
            <dl class="kv">
                <dt>(t("Endpoint"))</dt>
                <dd class="mono">(page.endpoint.unwrap_or("—"))</dd>
                <dt>(t("Region"))</dt>
                <dd class="mono">(garage::REGION)</dd>
                <dt>(t("Addressing"))</dt>
                <dd>(t("path-style: the bucket is the first part of the path"))</dd>
            </dl>
            <p class="field-hint">(t("Most clients address a bucket as a name of its own unless told \
                 otherwise. Each has a setting for this:"))</p>
            <table>
                <tbody>
                    <tr><td>"aws"</td><td class="mono">"aws configure set default.s3.addressing_style path"</td></tr>
                    <tr><td>"boto3"</td><td class="mono">"Config(signature_version=\"s3v4\", s3={\"addressing_style\": \"path\"})"</td></tr>
                    <tr><td>"@aws-sdk/client-s3"</td><td class="mono">"forcePathStyle: true, requestChecksumCalculation: \"WHEN_REQUIRED\""</td></tr>
                    <tr><td>"rclone"</td><td class="mono">"provider = Other, force_path_style = true"</td></tr>
                </tbody>
            </table>
            <p class="field-hint">(t("The other two settings are for presigned URLs. Unless told s3v4, \
                 boto3 signs one the old way, which the store reads as no signature at all. And the \
                 JavaScript SDK puts the checksum of an empty body into a presigned upload, so \
                 anything uploaded to it is refused as not matching."))</p>
        </section>

        <section class="card stack" id="buckets">
            <div class="split">
                <p class="card-label">(t("Buckets"))</p>
                @if let Some(disk) = page.disk {
                    <span class="who">(presets::size_field(page.reserved))" / "(presets::size_field(disk))</span>
                }
            </div>
            @if !page.running {
                <p class="tile-detail">(t("The store is not running, so what each bucket holds cannot \
                     be asked, and keys cannot be made."))</p>
            }
            @if page.buckets.is_empty() {
                <p class="tile-detail">(t("None yet. A bucket holds objects under keys, and is reached \
                     with a key of its own."))</p>
            } @else {
                <table>
                    <tbody>
                        @for (bucket, usage) in page.buckets {
                            <tr>
                                <td><a href=(format!("{base}/{}", bucket.name))>(bucket.name.as_str())</a></td>
                                <td class="tile-detail">(t("up to "))(presets::size_field(bucket.max_size))</td>
                                <td class="tile-detail">
                                    @if let Some(usage) = usage {
                                        (human(usage.bytes))" · "(usage.objects)(t(" objects"))
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
            @if page.may_deploy && page.service.is_ours() {
                <form method="post" action=(&base) class="stack">
                    <label for="bucket-name">(t("Name"))</label>
                    <input id="bucket-name" name="name" type="text" autocomplete="off" class="mono"
                           required placeholder="uploads">
                    <p class="field-hint">(t("3 to 63 lowercase letters, digits and hyphens, starting and \
                         ending with a letter or a digit."))</p>
                    <label for="bucket-size">(t("Holds up to"))</label>
                    <input id="bucket-size" name="max_size" type="text" autocomplete="off" required
                           placeholder="5 GB">
                    @if let Some(left) = left {
                        <p class="field-hint">(t("Left on this store's disk: "))(presets::size_field(left))</p>
                    }
                    <div class="actions">
                        <button class="btn btn-secondary" type="submit">(t("Create bucket"))</button>
                    </div>
                </form>
            }
        </section>
    }
}
