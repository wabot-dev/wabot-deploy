//! How an image gets here: the `docker push` this service expects, and
//! the token it needs.

use crate::features::services::texts::t;
use hypertext::prelude::*;

/// What the releases card can say about pushing to this service.
///
/// [`Push::To`] carries the same three parts the registry compares when
/// a manifest arrives — this node as the host, the repository name
/// without one, and the tag the service watches — so what the page
/// shows is what `release` will match rather than a second guess at it.
///
/// The other three are the ways a push cannot land here, and they are
/// why this is not an `Option`: printing nothing left somebody looking
/// at a card that says "push an image to this repository" above an
/// empty space, with no way to find out which repository or why there
/// wasn't one. A dead end is a value somebody can act on.
pub enum Push {
    /// Push here, and it becomes a release.
    To(String),
    /// The image belongs to another registry. A push to the service's
    /// own repository here still lands — the registry matches it by slug —
    /// and the service runs what was pushed from then on. Carries that
    /// reference.
    Elsewhere(String),
    /// Pinned by digest: it names bytes, and nothing watches bytes.
    Pinned,
    /// No domain, so this node's registry has no name to dial.
    Nameless,
}

pub fn push(
    service: &crate::domain::entities::service::Service,
    project_slug: &str,
    node_domain: Option<&str>,
) -> Option<Push> {
    // Managed images are the node's own choice, pinned to a major
    // version; nothing is ever pushed at one, and there is nothing
    // useful to say about that.
    if service.kind.is_managed() {
        return None;
    }

    let Ok(parsed) = crate::domain::values::image_reference::ImageReference::parse(&service.image)
    else {
        return Some(Push::Pinned);
    };
    let Some(host) = node_domain else {
        return Some(Push::Nameless);
    };
    // No image is a service waiting for its first push, and the create
    // form promises it the commands. It got "pinned by digest" instead:
    // `parts` has nothing to take apart in an empty reference, and that
    // was read as a digest. Found on a VM node.
    if parsed.is_empty() {
        return Some(Push::To(format!(
            "{host}/{project_slug}/{}:latest",
            service.slug
        )));
    }
    let Some(reference) = parsed.parts() else {
        return Some(Push::Pinned);
    };
    let tag = service.tracked_tag().unwrap_or_else(|| "latest".into());
    // **A star is a rule, not a tag.** `*` matches every tag on the way
    // in; it is not something anybody types after a colon, and a command
    // ending `:*` is one somebody pastes and watches fail. The example
    // shows `latest` — which the rule accepts along with everything else.
    let tag = match tag == crate::domain::entities::service::EVERY_TAG {
        true => "latest".to_string(),
        false => tag,
    };

    // A push to `<project>/<service>` is a release for that service
    // whatever its image field says — see `registry::manifests`. This
    // said the opposite for seven weeks after that changed, and told
    // somebody to edit a field the push would have set itself.
    let name = reference.name();
    match name == project_slug || name.starts_with(&format!("{project_slug}/")) {
        // The registry names a pushed image under the node's own host,
        // and that is what containerd is later asked to pull. A
        // reference with no host is one containerd reads as its first
        // segment being a registry to dial.
        true => Some(Push::To(format!("{host}/{name}:{tag}"))),
        // `latest`, and not the tag the other registry's image carries.
        // A service created from `nginx:alpine` was offered
        // `…/nubo-app:alpine` — the tag belongs to somebody else's
        // image, and inheriting it means every service made from a
        // public base image suggests a tag nobody would have chosen.
        false => Some(Push::Elsewhere(format!(
            "{host}/{project_slug}/{}:latest",
            service.slug
        ))),
    }
}

/// A search term, safe to put in a link.
///
/// `rsx!` escapes for HTML and that is a different job: a needle with a
/// `&` in it would end the query parameter and start another, and one
/// with a `#` would turn the rest into a fragment. Both are things
/// somebody searches a log for.
pub fn urlencode(text: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("q", text)
        .finish()
        .trim_start_matches("q=")
        .to_string()
}

/// The one command, with the name in full, on the tab that decides what
/// a push does.
///
/// The service's own page has the whole sequence — build, then push —
/// behind a disclosure, because there it is read once while setting the
/// service up. Here the question is narrower: the field above says which
/// tag is watched, and this says what to push at it.
///
/// Not translated, and the sentence around it is: it is pasted into a
/// terminal, and a terminal does not speak Spanish.
pub fn push_target<'a>(push: &'a Option<Push>, tokens: &'a str) -> impl Renderable + 'a {
    let (copy, copied) = (t("Copy"), t("Copied"));
    // **Both references are pushable, and that is the point.** A service
    // running somebody else's image still has a name here, and the
    // registry matches a push by `<project>/<service>` as well as by the
    // declared image — so `Elsewhere` carries a command that works today,
    // not an explanation of why there is none.
    //
    // That is what the paragraph here was hiding. Jorge said it made no
    // sense and it went, which left this card empty on exactly the service
    // he was looking at: `docker.io/library/nginx:alpine` in project
    // `coffe-store`. Empty was worse than the paragraph; the command is
    // better than both.
    let command = match push {
        Some(Push::To(target)) | Some(Push::Elsewhere(target)) => format!("docker push {target}"),
        _ => String::new(),
    };
    let replaces = matches!(push, Some(Push::Elsewhere(_)));
    let inner = rsx! {
        <div class="dsn stack-sm">
            @if !command.is_empty() {
                <div class="dsn-line">
                    <pre class="dsn-value" data-copy
                         data-copy-label=(copy) data-copied-label=(copied)>(&command)</pre>
                </div>
                // **The link is its own string.** An inline link splits a
                // sentence in two whatever you do, so the split is at a
                // phrase boundary and the space and the full stop are
                // literals outside `t` — a translated string that ends in
                // a space is one somebody trims by accident.
                //
                // Worded by Jorge, who called what was here rubbish: it
                // said "after docker login with a push token from the
                // project page" and left somebody to find that page.
                <p class="field-hint">
                    (t("An example of how to push to this service. First you have to log in with \
                        a token, which you can generate in"))
                    (" ")
                    <a href=(tokens)>(t("the project's settings"))</a>
                    (".")
                </p>
                // Six words, because pushing here does change what the
                // service runs and somebody should know before they do it.
                @if replaces {
                    <p class="field-hint">(t("This replaces the image above."))</p>
                }
            }
            // The dead ends, each said rather than left blank. A card
            // headed "what to push at" with nothing under it is one
            // somebody reads as broken rather than as not applicable.
            @if matches!(push, Some(Push::Pinned)) {
                <p class="field-hint">(t("This service is pinned to one image by digest, which \
                     names bytes rather than a name a push can move. Give it a tag above for \
                     pushes to reach it."))</p>
            }
            @if matches!(push, Some(Push::Nameless)) {
                <p class="field-hint">(t("This node has no domain, so its registry has no name a \
                     client could dial. Give the node one in its settings, and the command to \
                     push here appears."))</p>
            }
        </div>
    }
    .render()
    .into_inner();

    // XSS SAFETY: the markup above is this function's own, and the
    // reference inside it went through `rsx!`'s escaping.
    wabot::ui::hypertext::island(
        "copy",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}

/// Two commands that put an image on this service, and a button to take
/// them.
///
/// The commands are not translated and the sentence around them is —
/// they are pasted into a terminal, and a terminal does not speak
/// Spanish. Behind a `<details>` for the same reason the connection
/// strings are: it is read once when the service is set up and is in
/// the way every time after that.
pub fn push_example(push: &Push, auto_deploy: bool, settings: &str) -> impl Renderable {
    let (copy, copied) = (t("Copy"), t("Copied"));
    // `docker build -t <target> .` rather than a tag step with a
    // placeholder in it: a command with `<your image>` in the middle is
    // one somebody pastes and watches fail. This one is complete.
    // Both cases with a reference get the commands. The second one has
    // a step in front of it — point the service here first — and that
    // is a sentence, not a reason to withhold what to type: the card
    // showed the bare reference and Jorge went looking for the build
    // and the push that were not there.
    let target = match push {
        Push::To(target) | Push::Elsewhere(target) => target.as_str(),
        _ => "",
    };
    let commands = match target.is_empty() {
        true => String::new(),
        false => format!("docker build -t {target} .\ndocker push {target}"),
    };
    let settings = settings.to_string();
    let landing = matches!(push, Push::To(_) | Push::Elsewhere(_));
    let inner = rsx! {
        <details>
            <summary>(t("How to push"))</summary>
            <div class="dsn stack-sm">
                // The dead ends, each with the one thing that ends it.
                // Said rather than left blank: the line above this card
                // tells somebody to push an image to "this repository",
                // and until something changes there is none.
                @if matches!(push, Push::Elsewhere(_)) {
                    <p class="field-hint">(t("This service runs an image from another registry. \
                         A push to its own repository here lands on it all the same, and from \
                         then on it runs what was pushed."))</p>
                }
                @if !commands.is_empty() {
                    <p class="field-hint">(t("From the directory with the Dockerfile, after \
                         docker login with a push token from the project page."))</p>
                    <div class="dsn-line">
                        <pre class="dsn-value" data-copy
                             data-copy-label=(copy) data-copied-label=(copied)>(&commands)</pre>
                    </div>
                }
                @if landing {
                    @if auto_deploy {
                        <p class="field-hint">(t("The push deploys it. Any other tag is stored and \
                             changes nothing here."))</p>
                    } @else {
                        <p class="field-hint">(t("The push appears above as a release, and waits for \
                             somebody to deploy it. Any other tag is stored and changes nothing \
                             here."))</p>
                    }
                }
                @if matches!(push, Push::Pinned) {
                    <p class="field-hint">(t("This service is pinned to one image by digest, \
                         which names bytes rather than a name a push can move. Give it a tag in \
                         settings for pushes to reach it."))</p>
                    <p class="field-hint">
                        <a href=(&settings)>(t("Change the image in settings"))</a>
                    </p>
                }
                @if matches!(push, Push::Nameless) {
                    <p class="field-hint">(t("This node has no domain, so its registry has no \
                         name a client could dial. Give the node one in its settings, and the \
                         command to push here appears."))</p>
                }
            </div>
        </details>
    }
    .render()
    .into_inner();

    // XSS SAFETY: the markup above is this function's own, and the
    // reference inside it went through `rsx!`'s escaping.
    wabot::ui::hypertext::island(
        "copy",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}
