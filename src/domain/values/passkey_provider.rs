/// Which password manager or security key made a passkey, from the AAGUID
/// it wrote into the credential.
///
/// What somebody recognises is where a passkey lives — Apple Passwords,
/// Google Password Manager, 1Password — and not the browser it was made
/// in: two passkeys made in Safari read the same, and one made in Chrome
/// on a Mac may be in iCloud.
///
/// The names are the community's list, with the icons left out so the
/// binary does not carry every vendor's logo. All zeros is an
/// authenticator that would not say, and reads as unknown.
pub fn named(aaguid: &str) -> Option<&'static str> {
    let wanted = aaguid.trim().to_ascii_lowercase();
    if wanted.is_empty() || wanted == UNKNOWN {
        return None;
    }
    TABLE
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .find(|(id, _)| *id == wanted)
        .map(|(_, name)| name)
}

/// The AAGUID as the list spells it, from the sixteen bytes in the
/// credential.
pub fn spelled(bytes: &[u8; 16]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Two characters standing for a name where a logo would be: the start of
/// its first word, which keeps `iC` and `1P` recognisable.
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect()
}

const UNKNOWN: &str = "00000000-0000-0000-0000-000000000000";

const TABLE: &str = include_str!("passkey_providers.tsv");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_managers_people_have_are_named() {
        for (aaguid, name) in [
            ("fbfc3007-154e-4ecc-8c0b-6e020557d7bd", "Apple Passwords"),
            (
                "EA9B8D66-4D01-1D21-3CE4-B6B48CB575D4",
                "Google Password Manager",
            ),
            ("bada5566-a7aa-401f-bd96-45619a55120d", "1Password"),
            ("d548826e-79b4-db40-a3d8-11116f7e8349", "Bitwarden"),
            ("cb69481e-8ff7-4039-93ec-0a2729a154a8", "YubiKey 5 Series"),
        ] {
            assert_eq!(named(aaguid), Some(name), "{aaguid}");
        }
    }

    #[test]
    fn an_authenticator_that_would_not_say_is_unknown() {
        assert_eq!(named(UNKNOWN), None);
        assert_eq!(named(""), None);
        assert_eq!(named("not an aaguid"), None);
    }

    #[test]
    fn the_bytes_are_spelled_as_the_list_spells_them() {
        let bytes = [
            0xfb, 0xfc, 0x30, 0x07, 0x15, 0x4e, 0x4e, 0xcc, 0x8c, 0x0b, 0x6e, 0x02, 0x05, 0x57,
            0xd7, 0xbd,
        ];
        assert_eq!(spelled(&bytes), "fbfc3007-154e-4ecc-8c0b-6e020557d7bd");
        assert_eq!(spelled(&[0; 16]), UNKNOWN);
    }

    #[test]
    fn a_name_stands_in_for_its_logo_by_how_it_starts() {
        assert_eq!(initials("Apple Passwords"), "Ap");
        assert_eq!(initials("1Password"), "1P");
        assert_eq!(initials("iCloud Keychain"), "iC");
        assert_eq!(initials(""), "");
    }
}
