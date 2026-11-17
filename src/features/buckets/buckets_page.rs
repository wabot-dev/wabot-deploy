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
    /// What every bucket is a subdomain of.
    pub domain: Option<&'a str>,
    /// A bucket's address with `<name>` for its name, as a client reaches it.
    pub bucket_address: Option<&'a str>,
    /// With what the running store says each one holds, when it answered.
    pub buckets: &'a [(Bucket, Option<Usage>)],
    pub reserved: u64,
    pub disk: Option<u64>,
    pub running: bool,
    /// The node running the store, when that is not this one: what each
    /// bucket holds is then what it last reported.
    pub runs_on: Option<&'a str>,
    /// Absent when no backup here holds this store and none are sent
    /// anywhere: the form could only refuse.
    pub restore: Option<Restore<'a>>,
}

pub struct Restore<'a> {
    pub default_name: &'a str,
    /// The copy's name under this node's domain, with `<name>` for its own.
    pub derived: Option<&'a str>,
    /// Where this node sends its backups, when that is not this machine:
    /// with a schedule and a bucket there is nothing here at all.
    pub elsewhere: Option<&'a str>,
}

pub fn render<'a>(page: &'a Buckets<'a>) -> impl Renderable + 'a {
    let base = format!(
        "/projects/{}/services/{}/buckets",
        page.project.slug, page.service.slug
    );
    let left = page.disk.map(|disk| disk.saturating_sub(page.reserved));
    let endpoint = page.endpoint.unwrap_or("https://…");
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
                @if let Some(node) = page.runs_on {
                    <dt>(t("Runs on"))</dt>
                    <dd class="mono">(node)</dd>
                }
                <dt>(t("Region"))</dt>
                <dd class="mono">(garage::REGION)</dd>
                @if let Some(domain) = page.domain {
                    <dt>(t("Each bucket"))</dt>
                    <dd class="mono">"<bucket>."(domain)</dd>
                }
            </dl>
            <p class="field-hint">(t("A client given the endpoint reaches a bucket either at its own name \
                 or as the first part of the path, whichever it builds — aws and boto3 build the path, \
                 the JavaScript SDK the name — and both work. It needs the endpoint and the region and \
                 nothing else:"))</p>
            <table>
                <tbody>
                    <tr><td>"aws"</td><td class="mono">"aws configure set region "(garage::REGION)"; aws s3 ls --endpoint-url "(endpoint)</td></tr>
                    <tr><td>"boto3"</td><td class="mono">"boto3.client(\"s3\", endpoint_url=\""(endpoint)"\", region_name=\""(garage::REGION)"\")"</td></tr>
                    <tr><td>"@aws-sdk/client-s3"</td><td class="mono">"new S3Client({ endpoint: \""(endpoint)"\", region: \""(garage::REGION)"\", requestChecksumCalculation: \"WHEN_REQUIRED\" })"</td></tr>
                    <tr><td>"rclone"</td><td class="mono">"provider = Other, endpoint = "(endpoint)", region = "(garage::REGION)</td></tr>
                </tbody>
            </table>
            <p class="field-hint">(t("Requests are signed with signature version 4 only. One signed the \
                 old way, version 2, is refused with Amazon's own answer: The authorization mechanism \
                 you have provided is not supported. boto3 presigns that way only in Amazon's oldest \
                 regions, us-east-1 among them; told this region, it signs with version 4 by itself. A \
                 request signed for another region is answered with this one: aws and boto3 sign it \
                 again by themselves, and any other client, or a presigned URL, is refused."))</p>
            <p class="field-hint">(t("The JavaScript SDK puts the checksum of an empty body into a \
                 presigned upload, so anything uploaded to it is refused as not matching. \
                 requestChecksumCalculation above is what stops it."))</p>
        </section>

        <section class="card stack" id="buckets">
            <div class="split">
                <p class="card-label">(t("Buckets"))</p>
                @if let Some(disk) = page.disk {
                    <span class="who">(presets::size_field(page.reserved))" / "(presets::size_field(disk))</span>
                }
            </div>
            @if let Some(node) = page.runs_on {
                @if page.running {
                    <p class="tile-detail">(t("What each bucket holds is what "))<span class="mono">(node)</span>(t(" last reported."))</p>
                } @else {
                    <p class="tile-detail"><span class="mono">(node)</span>(t(" has not reported what each bucket holds yet."))</p>
                }
            } @else if !page.running {
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
                    @if let Some(domain) = page.domain {
                        <div class="name-under">
                            <input id="bucket-name" name="name" type="text" autocomplete="off"
                                   class="mono" required placeholder="uploads">
                            <span class="suffix">"."(domain)</span>
                        </div>
                        <p class="field-hint">(t("The name is the bucket's subdomain, and clients reach \
                             it at "))<span class="mono">(page.bucket_address.unwrap_or_default())</span>(t(". \
                             It has to point where the store's domain does, and is checked before the \
                             bucket is made: one record "))<span class="mono">"*."(domain)</span>(t(" covers \
                             every bucket of this store. 3 to 63 lowercase letters, digits and hyphens, \
                             starting and ending with a letter or a digit."))</p>
                    } @else {
                        <input id="bucket-name" name="name" type="text" autocomplete="off" class="mono"
                               required placeholder="uploads">
                        <p class="field-hint">(t("3 to 63 lowercase letters, digits and hyphens, starting and \
                             ending with a letter or a digit."))</p>
                    }
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

        @if let Some(restore) = &page.restore {
            <section class="stack" id="restore">
                <p class="card-label">(t("Restore into a copy"))</p>
                <form method="post"
                      action=(format!("/projects/{}/stores/{}/restore", page.project.slug, page.service.slug))
                      class="card stack">
                    <label for="restore-at">(t("Up to which moment"))</label>
                    <input id="restore-at" name="at" type="text" autocomplete="off" class="mono" value="">
                    <p class="field-hint">(t("UTC: 2026-08-16 14:32. The copy is the newest backup taken \
                         at or before it — nothing between two backups can be reached. Empty means the \
                         newest backup there is."))</p>

                    <label for="restore-into">(t("Call the copy"))</label>
                    <input id="restore-into" name="into" type="text" autocomplete="off" class="mono"
                           value="" placeholder=(restore.default_name)>
                    <p class="field-hint">(t("A new store in this project, with the original's buckets, \
                         rules and ceilings, a name and an address of its own, and none of its keys: a \
                         bucket's are made again on the copy, so no credential opens both. The original \
                         keeps running and keeps its objects."))</p>
                    <label for="restore-hostname">(t("Address"))</label>
                    @if let Some(derived) = restore.derived {
                        <input id="restore-hostname" name="hostname" type="text" autocomplete="off"
                               class="mono" placeholder=(derived)>
                        <p class="field-hint">(t("The name clients reach it at over HTTPS. Empty is the \
                             one shown, with the store's name in place of <name>. It has to point at \
                             this node."))</p>
                    } @else {
                        <input id="restore-hostname" name="hostname" type="text" autocomplete="off"
                               class="mono" required>
                        <p class="field-hint">(t("The name clients reach it at over HTTPS. This node has \
                             no domain to derive one under, so it has to be typed, and it has to point \
                             at this node."))</p>
                    }
                    <p class="field-hint">(t("It brings its buckets at its first deployment, which starts \
                         as soon as you press this, each named under the copy's address where that name \
                         points at this node and reached by its path where it does not. None of them is \
                         public: public reads stay with the original."))</p>
                    @if let Some(destination) = restore.elsewhere {
                        <p class="field-hint">(t("The newest backup is looked for both on this machine and at "))<span class="mono">(destination)</span>
                            (t(" — where a schedule sends them, and where they are the only copy once the \
                             transfer has succeeded. Downloading it is part of that first deployment, so a \
                             large one takes as long as it takes."))</p>
                    }
                    <div class="actions">
                        <button class="btn btn-secondary" type="submit">(t("Restore into a copy"))</button>
                    </div>
                </form>
            </section>
        }
    }
}
