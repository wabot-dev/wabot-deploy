/// Something that happened to an account, for its owner to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountEvent {
    PasskeyAdded,
    PasskeyRemoved,
    PasswordDeleted,
    DeviceTrusted,
    DeviceWithdrawn,
    RequestApproved,
    RequestRefused,
    LinkUsed,
    SignedOutEverywhere,
}

impl AccountEvent {
    pub const ALL: [Self; 9] = [
        Self::PasskeyAdded,
        Self::PasskeyRemoved,
        Self::PasswordDeleted,
        Self::DeviceTrusted,
        Self::DeviceWithdrawn,
        Self::RequestApproved,
        Self::RequestRefused,
        Self::LinkUsed,
        Self::SignedOutEverywhere,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PasskeyAdded => "passkey-added",
            Self::PasskeyRemoved => "passkey-removed",
            Self::PasswordDeleted => "password-deleted",
            Self::DeviceTrusted => "device-trusted",
            Self::DeviceWithdrawn => "device-withdrawn",
            Self::RequestApproved => "request-approved",
            Self::RequestRefused => "request-refused",
            Self::LinkUsed => "link-used",
            Self::SignedOutEverywhere => "signed-out-everywhere",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == text)
    }

    /// The words the Activity tab shows, in English: the page translates.
    pub fn label(self) -> &'static str {
        match self {
            Self::PasskeyAdded => "Passkey added",
            Self::PasskeyRemoved => "Passkey removed",
            Self::PasswordDeleted => "Password deleted",
            Self::DeviceTrusted => "Device trusted",
            Self::DeviceWithdrawn => "Device withdrawn",
            Self::RequestApproved => "Sign-in approved",
            Self::RequestRefused => "Sign-in refused",
            Self::LinkUsed => "Sign-in link used",
            Self::SignedOutEverywhere => "Signed out everywhere",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_reads_back_as_itself() {
        for event in AccountEvent::ALL {
            assert_eq!(AccountEvent::parse(event.as_str()), Some(event));
        }
    }
}
