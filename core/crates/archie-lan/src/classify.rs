//! What kind of thing a device is, from what it says about itself.
//!
//! The kinds are the device families ArchieOS has adapters for
//! ([`DeviceAdapterClass`]). A device that fits none is kept in the inventory without a
//! class: guessing "television" for something that only said "UPnP" would put a wrong fact
//! where an honest blank belongs.

use archie_types::DeviceAdapterClass;

/// Everything a device said that bears on what it is, lowercased.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    /// SSDP targets, the `SERVER` line, and the UPnP device type, manufacturer and model.
    pub upnp: Vec<String>,
    /// mDNS service kinds, e.g. `_ipp._tcp`.
    pub services: Vec<String>,
}

impl Evidence {
    fn says(&self, needle: &str) -> bool {
        self.upnp.iter().any(|text| text.contains(needle))
    }

    fn offers(&self, kinds: &[&str]) -> bool {
        self.services
            .iter()
            .any(|service| kinds.contains(&service.as_str()))
    }
}

/// The most specific family the evidence supports. Order matters: a television also runs
/// a media renderer, and a router also answers as a generic UPnP root device.
pub fn classify(evidence: &Evidence) -> Option<DeviceAdapterClass> {
    use DeviceAdapterClass::*;
    if evidence.offers(&["_home-assistant._tcp"]) {
        return Some(HomeAssistant);
    }
    if evidence.says("internetgatewaydevice")
        || evidence.says("wandevice")
        || evidence.says("wanipconnection")
        || evidence.says("wfadevice")
    {
        return Some(Router);
    }
    if evidence.says("printer")
        || evidence.offers(&[
            "_ipp._tcp",
            "_ipps._tcp",
            "_printer._tcp",
            "_pdl-datastream._tcp",
        ])
    {
        return Some(Printer);
    }
    if evidence.says("networkcamera")
        || evidence.says("securitycamera")
        || evidence.offers(&["_axis-video._tcp", "_rtsp._tcp"])
    {
        return Some(Camera);
    }
    if evidence.says("roku")
        || evidence.says("dial-multiscreen")
        || evidence.says("mediarenderer")
        || evidence.offers(&[
            "_googlecast._tcp",
            "_androidtvremote2._tcp",
            "_airplay._tcp",
            "_amzn-wplay._tcp",
        ])
    {
        return Some(Television);
    }
    if evidence.offers(&["_apple-mobdev2._tcp"]) {
        return Some(Mobile);
    }
    if evidence.says("mediaserver")
        || evidence.offers(&[
            "_ssh._tcp",
            "_sftp-ssh._tcp",
            "_smb._tcp",
            "_workstation._tcp",
            "_nfs._tcp",
        ])
    {
        return Some(Host);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upnp(texts: &[&str]) -> Evidence {
        Evidence {
            upnp: texts.iter().map(|t| t.to_lowercase()).collect(),
            services: Vec::new(),
        }
    }

    fn mdns(kinds: &[&str]) -> Evidence {
        Evidence {
            upnp: Vec::new(),
            services: kinds.iter().map(|k| k.to_string()).collect(),
        }
    }

    #[test]
    fn a_device_is_the_most_specific_thing_it_says_it_is() {
        assert_eq!(
            classify(&upnp(&["roku:ecp", "Roku/12.5.0 UPnP/1.0"])),
            Some(DeviceAdapterClass::Television)
        );
        assert_eq!(
            classify(&upnp(&[
                "upnp:rootdevice",
                "urn:schemas-upnp-org:device:InternetGatewayDevice:1"
            ])),
            Some(DeviceAdapterClass::Router)
        );
        assert_eq!(
            classify(&mdns(&["_ipp._tcp", "_http._tcp"])),
            Some(DeviceAdapterClass::Printer)
        );
        assert_eq!(
            classify(&mdns(&["_googlecast._tcp"])),
            Some(DeviceAdapterClass::Television)
        );
        assert_eq!(
            classify(&mdns(&["_home-assistant._tcp", "_ssh._tcp"])),
            Some(DeviceAdapterClass::HomeAssistant)
        );
        assert_eq!(
            classify(&mdns(&["_smb._tcp"])),
            Some(DeviceAdapterClass::Host)
        );
        assert_eq!(
            classify(&mdns(&["_apple-mobdev2._tcp"])),
            Some(DeviceAdapterClass::Mobile)
        );
    }

    #[test]
    fn a_device_that_says_too_little_is_left_without_a_kind() {
        assert_eq!(
            classify(&upnp(&["upnp:rootdevice", "Linux/5.4 UPnP/1.0"])),
            None
        );
        assert_eq!(classify(&mdns(&["_http._tcp", "_raop._tcp"])), None);
        assert_eq!(classify(&Evidence::default()), None);
    }

    #[test]
    fn companion_link_alone_does_not_identify_a_mobile_device() {
        assert_eq!(classify(&mdns(&["_companion-link._tcp"])), None);
    }
}
