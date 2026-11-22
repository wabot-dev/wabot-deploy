/// What to call a browser: `Safari on macOS`, from its `User-Agent`.
///
/// Order matters, because every engine claims to be the ones before it:
/// Edge says Chrome and Safari, Chrome says Safari, and an iPad on iPadOS
/// says it is a Mac.
pub fn from_user_agent(agent: &str) -> String {
    let browser = if agent.contains("Edg/") || agent.contains("EdgA/") || agent.contains("EdgiOS/")
    {
        "Edge"
    } else if agent.contains("Firefox/") || agent.contains("FxiOS/") {
        "Firefox"
    } else if agent.contains("OPR/") {
        "Opera"
    } else if agent.contains("Chrome/") || agent.contains("CriOS/") {
        "Chrome"
    } else if agent.contains("Safari/") {
        "Safari"
    } else {
        "A browser"
    };
    let system = if agent.contains("iPhone") {
        "iPhone"
    } else if agent.contains("iPad") {
        "iPad"
    } else if agent.contains("Android") {
        "Android"
    } else if agent.contains("Windows") {
        "Windows"
    } else if agent.contains("Mac OS X") || agent.contains("Macintosh") {
        "macOS"
    } else if agent.contains("CrOS") {
        "ChromeOS"
    } else if agent.contains("Linux") {
        "Linux"
    } else {
        return browser.to_string();
    };
    format!("{browser} on {system}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_browser_is_named_for_what_it_is_not_what_it_claims() {
        for (agent, name) in [
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15",
                "Safari on macOS",
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0",
                "Edge on Windows",
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
                "Safari on iPhone",
            ),
            (
                "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
                "Chrome on Android",
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0",
                "Firefox on Linux",
            ),
            ("curl/8.7.1", "A browser"),
        ] {
            assert_eq!(from_user_agent(agent), name);
        }
    }
}
