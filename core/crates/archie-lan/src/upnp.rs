//! What a UPnP device says about itself in the description it publishes.

/// The fields worth keeping from a device description.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Description {
    pub friendly_name: String,
    pub manufacturer: String,
    pub model_name: String,
    pub model_number: String,
    /// e.g. `urn:schemas-upnp-org:device:InternetGatewayDevice:1`.
    pub device_type: String,
}

/// The text of the first `<tag>…</tag>` in `xml`, unescaped and cut to a line's length.
/// These documents are flat lists of short fields from devices of every quality, so the
/// reader looks for the tag and does not try to be an XML parser.
pub fn first(xml: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let Some(start) = xml.find(&open).map(|at| at + open.len()) else {
        return String::new();
    };
    let Some(end) = xml[start..].find(&format!("</{tag}>")) else {
        return String::new();
    };
    let raw: String = xml[start..start + end].chars().take(200).collect();
    let text = raw
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    // One line of printable text, whatever the device put there.
    text.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn parse(xml: &[u8]) -> Description {
    let xml = String::from_utf8_lossy(xml);
    Description {
        friendly_name: first(&xml, "friendlyName"),
        manufacturer: first(&xml, "manufacturer"),
        model_name: first(&xml, "modelName"),
        model_number: first(&xml, "modelNumber"),
        device_type: first(&xml, "deviceType"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTER: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <device>
    <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
    <friendlyName>Casa &amp; Taller</friendlyName>
    <manufacturer>TP-Link</manufacturer>
    <modelName>Archer AX55</modelName>
    <modelNumber>1.0</modelNumber>
    <deviceList><device><friendlyName>WAN</friendlyName></device></deviceList>
  </device>
</root>"#;

    #[test]
    fn a_description_gives_the_devices_own_name_and_kind() {
        let description = parse(ROUTER.as_bytes());
        assert_eq!(
            description.friendly_name, "Casa & Taller",
            "the root device's name, unescaped"
        );
        assert_eq!(description.manufacturer, "TP-Link");
        assert_eq!(description.model_name, "Archer AX55");
        assert!(description.device_type.contains("InternetGatewayDevice"));
    }

    #[test]
    fn a_broken_or_hostile_description_yields_short_plain_text_or_nothing() {
        assert_eq!(parse(b"not xml at all"), Description::default());
        assert_eq!(first("<modelName>unterminated", "modelName"), "");
        let long = format!("<friendlyName>{}</friendlyName>", "a".repeat(5000));
        assert_eq!(first(&long, "friendlyName").len(), 200);
        assert_eq!(
            first(
                "<friendlyName>TV\u{1b}[31m\nsala</friendlyName>",
                "friendlyName"
            ),
            "TV[31msala"
        );
    }
}
