//! Who is in a project, and what each of them may do.

use hypertext::prelude::*;

use crate::accounts::roles::ProjectRole;
use crate::policies::access;
use crate::views::texts::language::t;

/// Who is in this project, and how to change that.
///
/// A section of the settings page rather than a page of its own. It was
/// three unlabelled columns and a wide row of fields under a heading
/// that said "People" — a table with no headers reads as a list of
/// facts nobody explained, and the role was a word floating beside a
/// name with nothing saying what it was.
///
/// So: headers, the roles said in a sentence under the form, and the
/// add form as a labelled row rather than three controls in a line.
pub fn people_card<'a>(
    here: &'a str,
    members: &'a [access::Member],
    allowed: crate::accounts::roles::Access,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("People"))</p>
            @if members.is_empty() {
                <p class="tile-detail">(t("Nobody yet. Administrators of this node reach every project \
                     without being added to it."))</p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("Person"))</th>
                            <th>(t("Can"))</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for member in members {
                            <tr>
                                <td>(&member.username)</td>
                                <td>
                                    <span class="badge">(member.role.label())</span>
                                    <span class="tile-detail">(crate::views::components::people_card::role_means(member.role))</span>
                                </td>
                                <td class="row-actions">
                                    @if allowed.may_administer() {
                                        <form method="post"
                                              action=(format!(
                                                  "{here}/people/{}/remove", member.account_id
                                              ))>
                                            <button class="btn btn-ghost destructive btn-sm"
                                                    type="submit">(t("Remove"))</button>
                                        </form>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }

            @if allowed.may_administer() {
                <form method="post" action=(format!("{here}/people")) class="stack-sm">
                    <div class="add-person">
                        <div>
                            <label for="member">(t("Username"))</label>
                            // Named `member`, not `username`, and with
                            // no placeholder repeating the word.
                            //
                            // `autocomplete="off"` is a request, and
                            // Safari refuses it here: it decides a field
                            // is a login from the *name, id and
                            // placeholder*, and all three said
                            // `username`. So the keychain offered the
                            // operator their own account for a field
                            // asking who to add — reported twice, with
                            // the arrow pointing at it.
                            //
                            // The name is also the truer one. This is
                            // not your username; it is the person you
                            // are adding.
                            <input id="member" name="member" type="text" autocomplete="off"
                                   required>
                        </div>
                        <div>
                            <label for="role">(t("Can"))</label>
                            <select id="role" name="role">
                                @for role in ProjectRole::ALL {
                                    <option value=(role.as_str())>(role.label())</option>
                                }
                            </select>
                        </div>
                        <button type="submit">(t("Add"))</button>
                    </div>
                    <p class="field-hint">(t("Somebody who already has an account on this node — adding \
                         them here does not create one. A new person is invited from \
                         Settings, People."))</p>
                </form>
            }
        </section>
    }
}

/// What a role lets somebody do, in the words the page can afford.
pub fn role_means(role: ProjectRole) -> &'static str {
    match role {
        ProjectRole::Owner => "everything here, including who else is in it",
        ProjectRole::Deployer => "deploy and change services",
        ProjectRole::Viewer => "look, and change nothing",
    }
}
