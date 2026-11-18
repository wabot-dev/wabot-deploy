use hypertext::prelude::*;

use crate::domain::policies::images::KEPT_BY_DEFAULT;
use crate::features::node::texts::t;

pub fn images_kept_card(kept: Option<usize>) -> impl Renderable {
    let value = kept.map(|kept| kept.to_string()).unwrap_or_default();
    let default = KEPT_BY_DEFAULT.to_string();

    rsx! {
        <section class="card stack" id="images">
            <p class="card-label">(t("Images kept"))</p>
            <form method="post" action="/node/images" class="stack">
                <label for="images-kept">(t("Per service"))</label>
                <input id="images-kept" name="kept" type="number" min="1" step="1"
                       inputmode="numeric" autocomplete="off"
                       placeholder=(&default) value=(&value)>
                <p class="field-hint">(t("The image a service runs and the newest one stay \
                     whatever the number. The rest go, oldest first, and their versions and \
                     tags with them. Empty means the default of two."))</p>
                <p class="field-hint">(t("A push holds its own bytes for a day, so an image \
                     pushed less than a day ago gives its disk back within the hour after its \
                     push is a day old."))</p>
                <div class="actions">
                    <button type="submit">(t("Save"))</button>
                </div>
            </form>
        </section>
    }
}
