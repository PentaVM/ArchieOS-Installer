//! Which conversation an event belongs to, for routing it to a context domain.
//!
//! A WhatsApp chat has a stable address, its JID: `<number>@s.whatsapp.net` for a person
//! (`@c.us` in WhatsApp Web), `<id>@g.us` for a group, `<id>@lid` for a hidden number.
//! Its title is a display name: not unique, and in a group anyone can change it. Only a
//! JID may move an event out of its source's own domain, so renaming a group can never
//! file personal messages under work, or the other way round.

/// The canonical form of a conversation id that may route, or `None` for anything that is
/// only a label. Collectors send labels as `title:<name>`; those never route.
pub fn routing_key(chat_id: &str) -> Option<String> {
    let (user, server) = chat_id.trim().split_once('@')?;
    // A device suffix (`<number>:<device>@…`) names one of the person's devices, not
    // another conversation.
    let user = user.split_once(':').map_or(user, |(user, _)| user);
    let server = match server.to_ascii_lowercase().as_str() {
        "s.whatsapp.net" | "c.us" => "s.whatsapp.net",
        "g.us" => "g.us",
        "lid" => "lid",
        _ => return None,
    };
    let valid = !user.is_empty()
        && user.len() <= 64
        && user.bytes().all(|b| b.is_ascii_digit() || b == b'-');
    valid.then(|| format!("{user}@{server}"))
}

#[cfg(test)]
mod tests {
    use super::routing_key;

    #[test]
    fn only_stable_conversation_ids_route() {
        assert_eq!(
            routing_key("34600000000@c.us").as_deref(),
            Some("34600000000@s.whatsapp.net")
        );
        assert_eq!(
            routing_key("34600000000:12@s.whatsapp.net").as_deref(),
            Some("34600000000@s.whatsapp.net")
        );
        assert_eq!(
            routing_key("120363042000000000@G.US").as_deref(),
            Some("120363042000000000@g.us")
        );
        for label in [
            "Family",
            "title:Trabajo",
            "Trabajo@g.us",
            "@g.us",
            "1@example.com",
            "",
        ] {
            assert_eq!(routing_key(label), None, "{label}");
        }
    }
}
