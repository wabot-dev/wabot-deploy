use hypertext::prelude::*;

use crate::features::dns::texts::t;
use crate::features::dns::zone_response::{Delivery, ServerChoice, ZoneResponse};

/// Who answers for the zone, chosen from the nodes that offer to.
///
/// A node that has not agreed is listed and cannot be ticked: "there is
/// nobody" and "I did not ask them" are different answers, and only one of
/// them is fixed from this page.
pub fn servers_card<'a>(here: &'a str, zone: &'a ZoneResponse) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("Which nodes answer for it"))</p>
            <form method="post" action=(format!("{here}/servers")) class="stack">
                @for choice in &zone.servers {
                    <label class="capability">
                        @if !choice.agreed {
                            <input type="checkbox" name=(format!("server-{}", choice.node_id)) value="1" disabled>
                        } @else if choice.chosen {
                            <input type="checkbox" name=(format!("server-{}", choice.node_id)) value="1" checked>
                        } @else {
                            <input type="checkbox" name=(format!("server-{}", choice.node_id)) value="1">
                        }
                        <span>
                            <strong>(&choice.node_name)</strong>
                            <span class="tile-detail">
                                @if let Some(name) = &choice.published {
                                    (name)
                                } @else {
                                    (t("no name the world can ask"))
                                }
                                " · "
                                (state(choice))
                            </span>
                        </span>
                    </label>
                }
                <p class="field-hint">(t("A node is offered here once it answers DNS and lets this node use it as a name server — it grants dns from its own console. Each is listed at the registrar under its own name."))</p>
                <div class="actions">
                    <button type="submit">(t("Save"))</button>
                </div>
            </form>
        </section>
    }
}

fn state(choice: &ServerChoice) -> String {
    if choice.this_node {
        return t("this node").to_string();
    }
    if !choice.agreed {
        return t("has not agreed to answer DNS for this node").to_string();
    }
    match &choice.last {
        _ if !choice.chosen => t("not chosen").to_string(),
        None => t("nothing sent yet").to_string(),
        Some(Delivery::Waiting) => t("sent, waiting to be collected").to_string(),
        Some(Delivery::Answering) => t("answering").to_string(),
        Some(Delivery::Withdrawn) => t("told to stop").to_string(),
        Some(Delivery::Refused(reason)) => format!("{} {reason}", t("refused:")),
    }
}
