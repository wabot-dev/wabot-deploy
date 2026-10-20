use hypertext::prelude::*;

use crate::domain::entities::dns_zone::DelegationStatus;
use crate::features::dns::texts::{delegated_elsewhere, delegated_partly, t};
use crate::features::dns::zone_response::ZoneResponse;
use crate::shell::layouts::layout;

/// Whether the world asks these nodes about the domain: what the parent
/// zone said when it was last asked, and a way to ask again now.
pub fn delegation_card<'a>(here: &'a str, zone: &'a ZoneResponse) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Where the registrar sends resolvers"))</p>
                <form method="post" action=(format!("{here}/check"))>
                    <button class="btn btn-secondary" type="submit">(t("Check now"))</button>
                </form>
            </div>
            @match &zone.delegation {
                None => {
                    <p class="tile-detail">(t("Not checked yet. It is asked every half hour, or now with the button."))</p>
                }
                Some(seen) => {
                    <p>
                        @match seen.status {
                            DelegationStatus::Here => {
                                <span class="badge badge-success"><span class="dot dot-success"></span>(t("Delegated here"))</span>
                            }
                            DelegationStatus::Partly => {
                                <span class="badge badge-warning"><span class="dot dot-warning"></span>(t("Partly"))</span>
                            }
                            DelegationStatus::Elsewhere => {
                                <span class="badge badge-danger"><span class="dot dot-danger"></span>(t("Points elsewhere"))</span>
                            }
                            DelegationStatus::Unregistered => {
                                <span class="badge badge-danger"><span class="dot dot-danger"></span>(t("Not registered"))</span>
                            }
                            DelegationStatus::Unknown => {
                                <span class="badge"><span class="dot dot-info"></span>(t("Not known"))</span>
                            }
                        }
                    </p>
                    <p class="tile-detail">
                        @match seen.status {
                            DelegationStatus::Here => { (t("The registrar sends resolvers to these nodes, and only these.")) }
                            DelegationStatus::Partly => { (delegated_partly(&seen.delegated.join(", "))) }
                            DelegationStatus::Elsewhere => { (delegated_elsewhere(&seen.delegated.join(", "))) }
                            DelegationStatus::Unregistered => {
                                (t("The parent zone has never heard of this domain. Is it registered, and is the registration paid?"))
                            }
                            DelegationStatus::Unknown => {
                                (t("It could not be asked:")) " " (seen.reason.as_deref().unwrap_or_default())
                            }
                        }
                    </p>
                    <p class="field-hint">(t("Last asked")) " " (layout::when(seen.checked_at))</p>
                }
            }
        </section>
    }
}
