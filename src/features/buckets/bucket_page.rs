//! One bucket: what it holds, its keys, and its rules.

use hypertext::prelude::*;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::bucket_key::{Access, BucketKey};
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::domain::policies::presets;
use crate::features::buckets::texts::t;
use crate::features::services::service_tabs_component::{service_tabs, ServicePlace};
use crate::infra::garage::Usage;
use crate::infra::node::memory::human;
use crate::shell::layouts::layout;

pub struct BucketView<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub bucket: &'a Bucket,
    pub keys: &'a [BucketKey],
    pub usage: Option<&'a Usage>,
    /// The node running the store, when that is not this one.
    pub runs_on: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    pub public_address: Option<&'a str>,
    pub derived_public: Option<&'a str>,
    pub may_deploy: bool,
    pub error: Option<&'a str>,
}

fn access_label(access: Access) -> &'static str {
    match access {
        Access::ReadWrite => t("Reads and writes"),
        Access::ReadOnly => t("Reads only"),
    }
}

fn days(value: Option<u32>) -> String {
    value.map(|days| days.to_string()).unwrap_or_default()
}

pub fn render<'a>(page: &'a BucketView<'a>) -> impl Renderable + 'a {
    let bucket = page.bucket;
    let here = format!(
        "/projects/{}/services/{}/buckets/{}",
        page.project.slug, page.service.slug, bucket.name
    );
    let editable = page.may_deploy && page.service.is_ours();
    let origins = bucket
        .cors_origins
        .iter()
        .map(|origin| origin.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    rsx! {
    (layout::style_tag())
        (service_tabs(&page.project.slug, page.service, page.may_deploy, ServicePlace::Buckets))
        @if let Some(message) = &page.error {
            (layout::error_note(message))
        }

        <section class="card stack" id="bucket">
            <div class="split">
                <h2>(bucket.name.as_str())</h2>
                <a class="btn btn-ghost btn-sm"
                   href=(format!("/projects/{}/services/{}/buckets", page.project.slug, page.service.slug))>
                    (t("All buckets"))
                </a>
            </div>
            <dl class="kv">
                @if let Some(endpoint) = page.endpoint {
                    <dt>(t("Address"))</dt>
                    <dd class="mono">(endpoint)"/"(bucket.name.as_str())</dd>
                }
                <dt>(t("Holds up to"))</dt>
                <dd>(presets::size_field(bucket.max_size))</dd>
                @if let Some(usage) = page.usage {
                    <dt>(t("Holds"))</dt>
                    <dd>(human(usage.bytes))" · "(usage.objects)(t(" objects"))</dd>
                    <dt>(t("Unfinished uploads"))</dt>
                    <dd>(usage.unfinished_uploads)</dd>
                }
            </dl>
            @match (page.usage.is_some(), page.runs_on) {
                (true, Some(node)) => {
                    <p class="tile-detail">(t("What it holds is what "))<span class="mono">(node)</span>(t(" last reported."))</p>
                }
                (false, Some(node)) => {
                    <p class="tile-detail"><span class="mono">(node)</span>(t(" has not reported what it holds yet."))</p>
                }
                (false, None) => {
                    <p class="tile-detail">(t("The store is not running, so what it holds cannot be asked."))</p>
                }
                (true, None) => {}
            }
        </section>

        <section class="card stack" id="keys">
            <p class="card-label">(t("Keys"))</p>
            @if page.keys.is_empty() {
                <p class="tile-detail">(t("None. A key opens this bucket and no other, and its secret \
                     is shown once, when it is made."))</p>
            } @else {
                <table>
                    <tbody>
                        @for key in page.keys {
                            <tr>
                                <td>(&key.name)</td>
                                <td class="mono">(&key.access_key_id)</td>
                                <td class="tile-detail">(access_label(key.access))</td>
                                <td>
                                    @if editable {
                                        <form method="post"
                                              action=(format!("{here}/keys/{}/revoke", key.id))>
                                            <button class="btn btn-ghost destructive btn-sm"
                                                    type="submit">(t("Revoke"))</button>
                                        </form>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
            @if editable {
                <form method="post" action=(format!("{here}/keys")) class="stack">
                    <label for="key-name">(t("What it is for"))</label>
                    <input id="key-name" name="name" type="text" autocomplete="off" required
                           placeholder="web">
                    <label for="key-access">(t("It may"))</label>
                    <select id="key-access" name="access">
                        <option value="read_write" selected>(t("Reads and writes"))</option>
                        <option value="read_only">(t("Reads only"))</option>
                    </select>
                    <div class="actions">
                        <button class="btn btn-secondary" type="submit">(t("Create key"))</button>
                    </div>
                </form>
            }
        </section>

        <section class="card stack" id="public">
            <p class="card-label">(t("Public read"))</p>
            @if let Some(address) = page.public_address {
                <dl class="kv">
                    <dt>(t("Read at"))</dt>
                    <dd class="mono">(address)"/<key>"</dd>
                </dl>
                <p class="tile-detail">(t("Anyone with this address reads an object by its key, \
                     with no key of their own. It does not list what the bucket holds."))</p>
                @if editable {
                    <form method="post" action=(format!("{here}/private"))>
                        <button class="btn btn-secondary" type="submit">(t("Make private"))</button>
                    </form>
                }
            } @else {
                <p class="tile-detail">(t("Private: every request is signed with one of its keys."))</p>
                @if editable {
                    <form method="post" action=(format!("{here}/public")) class="stack">
                        <label for="public-name">(t("Name it is read at"))</label>
                        @if let Some(derived) = page.derived_public {
                            <input id="public-name" name="hostname" type="text" autocomplete="off"
                                   class="mono" placeholder=(derived)>
                        } @else {
                            <input id="public-name" name="hostname" type="text" autocomplete="off"
                                   class="mono" required>
                        }
                        <p class="field-hint">(t("Empty is the one shown. It has to point at this \
                             node, and the store's allowlist applies to it as to the store's own \
                             name. A path that ends in / answers its index.html, and nothing \
                             answers a listing."))</p>
                        <div class="actions">
                            <button class="btn btn-secondary" type="submit">(t("Make public"))</button>
                        </div>
                    </form>
                }
            }
        </section>

        @if editable {
            <section class="card stack" id="rules">
                <p class="card-label">(t("Rules"))</p>
                <form method="post" action=(format!("{here}/rules")) class="stack">
                    <label for="max-size">(t("Holds up to"))</label>
                    <input id="max-size" name="max_size" type="text" autocomplete="off" required
                           value=(presets::size_field(bucket.max_size))>
                    <p class="field-hint">(t("A write that would take it past this is refused with an \
                         error the client reads. The store's buckets together have to fit its disk."))</p>

                    <label for="cors">(t("Pages that may reach it from a browser"))</label>
                    <textarea id="cors" name="cors_origins" rows="3" class="mono" autocomplete="off"
                              placeholder="https://app.example.com">(origins)</textarea>
                    <p class="field-hint">(t("One origin per line, like https://app.example.com; \
                         https://*.example.com for every name under it, or * for any page at all. \
                         Without one, an upload from a browser fails there and works in curl."))</p>

                    <label for="expire">(t("Delete objects after, in days"))</label>
                    <input id="expire" name="expire_days" type="text" inputmode="numeric"
                           autocomplete="off" value=(days(bucket.expire_days))>
                    <label for="abort">(t("Abandon unfinished uploads after, in days"))</label>
                    <input id="abort" name="abort_uploads_days" type="text" inputmode="numeric"
                           autocomplete="off" value=(days(bucket.abort_uploads_days))>
                    <p class="field-hint">(t("Empty is never. An upload in parts that nobody finished \
                         holds disk and appears in no listing. The store looks once a day, at midnight \
                         UTC, so an object goes within a day of its time, not at it."))</p>
                    <div class="actions">
                        <button class="btn btn-secondary" type="submit">(t("Save"))</button>
                    </div>
                </form>
            </section>

            <section class="card stack" id="delete">
                <p class="card-label">(t("Danger zone"))</p>
                <p class="tile-detail">(t("A bucket is deleted only when it is empty, and its keys stop \
                     working with it. Emptying one is a client's job: the store refuses while anything \
                     is in it."))</p>
                <form method="post" action=(format!("{here}/delete")) class="stack">
                    <label for="confirm">(t("Type the name to confirm: "))(bucket.name.as_str())</label>
                    <input id="confirm" name="confirm" type="text" autocomplete="off"
                           class="mono" required placeholder=(bucket.name.as_str())>
                    <button class="btn btn-danger" type="submit">(t("Delete bucket"))</button>
                </form>
            </section>
        }
    }
}
