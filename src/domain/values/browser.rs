/// Who is at the door, as far as a request can say: the host it was sent
/// to, which is what a passkey is bound to; the device cookie it carried;
/// what to call it; and where it came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Browser {
    pub host: String,
    pub device: Option<String>,
    pub name: String,
    pub address: Option<String>,
}

/// A passkey's answer to a challenge the page rendered. Empty when the
/// browser gave none, which is what a browser without scripting gives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ceremony {
    pub challenge: String,
    pub credential: String,
}

impl Ceremony {
    pub fn given(&self) -> bool {
        !self.credential.is_empty()
    }
}
