//! AT-SPI, read the way a screen reader reads it.
//!
//! The session bus says where the accessibility bus is; that bus has a registry whose
//! children are the applications with something to declare. Each is a D-Bus connection, so
//! the bus itself says which process it belongs to, which is how a window on the stage and
//! an application here are known to be the same one.
//!
//! The walk is bounded in depth and in objects. An application's tree is its own account of
//! itself, and nothing here trusts it further than reading it.

use crate::{kind_of_role, Control, DeclareError};
use archie_iris::Rect;
use zbus::names::BusName;
use zbus::zvariant::OwnedObjectPath;
use zbus::Connection;

const ROOT: &str = "/org/a11y/atspi/accessible/root";
/// How deep into a window's tree to go, and how many objects to read at all. A window is a
/// handful of controls; an unbounded walk is something else's denial of service.
const MAX_DEPTH: usize = 12;
const MAX_OBJECTS: usize = 2_000;
/// AT-SPI's coordinates relative to the window, which are the surface's own.
const WINDOW_COORDS: u32 = 1;

#[zbus::proxy(interface = "org.a11y.atspi.Accessible", assume_defaults = false)]
trait Accessible {
    fn get_children(&self) -> zbus::Result<Vec<(String, OwnedObjectPath)>>;
    fn get_role_name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
}

#[zbus::proxy(interface = "org.a11y.atspi.Component", assume_defaults = false)]
trait Component {
    fn get_extents(&self, coordinates: u32) -> zbus::Result<(i32, i32, i32, i32)>;
}

/// A connection to the accessibility bus, if a session has one.
pub struct Accessibility {
    bus: Connection,
}

impl Accessibility {
    /// Asks the session bus where the accessibility bus is, and connects to it.
    pub async fn connect() -> Result<Self, DeclareError> {
        let session = Connection::session()
            .await
            .map_err(|_| DeclareError::NoBus)?;
        let address: String = session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            )
            .await
            .map_err(|_| DeclareError::NoBus)?
            .body()
            .deserialize()
            .map_err(|_| DeclareError::NoBus)?;
        let bus = zbus::connection::Builder::address(address.as_str())
            .map_err(|e| DeclareError::Bus(e.to_string()))?
            .build()
            .await
            .map_err(|e| DeclareError::Bus(e.to_string()))?;
        Ok(Self { bus })
    }

    /// Every control the application running as `pid` declares in the window of that
    /// size, in window coordinates.
    ///
    /// The size is how one window of an application is told from another: coordinates
    /// from the wrong window would be a click in the wrong place. `None` reads every
    /// window, which is for looking, not for acting.
    pub async fn controls_of(
        &self,
        pid: u32,
        window: Option<(i32, i32)>,
    ) -> Result<Vec<Control>, DeclareError> {
        let names = zbus::fdo::DBusProxy::new(&self.bus)
            .await
            .map_err(|e| DeclareError::Bus(e.to_string()))?;
        let registry = self.accessible("org.a11y.atspi.Registry", ROOT).await?;
        let applications = registry
            .get_children()
            .await
            .map_err(|e| DeclareError::Bus(e.to_string()))?;

        let mut controls = Vec::new();
        let mut read = 0_usize;
        for (service, path) in applications {
            let Ok(name) = BusName::try_from(service.clone()) else {
                continue;
            };
            if names.get_connection_unix_process_id(name).await.ok() != Some(pid) {
                continue;
            }
            // With one window there is nothing to tell apart, and asking how big it is
            // would be another round trip for an answer that cannot change anything.
            let mut windows = 0;
            let mut pending = vec![(service.clone(), path, 0_usize)];
            while let Some((service, path, depth)) = pending.pop() {
                read += 1;
                if read > MAX_OBJECTS {
                    return Ok(controls);
                }
                let accessible = self.accessible(&service, path.as_str()).await?;
                let role = accessible.get_role_name().await.unwrap_or_default();
                if role == "frame" && windows > 1 {
                    // Another window of the same application measures differently, and
                    // its coordinates would be a click in the wrong place.
                    match (window, self.bounds(&service, path.as_str()).await) {
                        (Some((width, height)), Some(bounds))
                            if bounds.width as i32 != width || bounds.height as i32 != height =>
                        {
                            continue;
                        }
                        (Some(_), None) => continue,
                        _ => {}
                    }
                }
                if let Some(kind) = kind_of_role(&role) {
                    if let Ok(name) = accessible.name().await {
                        if let Some(bounds) = self.bounds(&service, path.as_str()).await {
                            controls.push(Control {
                                name,
                                kind,
                                bounds,
                                path: path.as_str().to_string(),
                            });
                        }
                    }
                }
                if depth >= MAX_DEPTH {
                    continue;
                }
                let children = accessible.get_children().await.unwrap_or_default();
                if depth == 0 {
                    windows = children.len();
                }
                for (child_service, child_path) in children {
                    pending.push((child_service, child_path, depth + 1));
                }
            }
            return Ok(controls);
        }
        Err(DeclareError::NoApplication(pid))
    }

    async fn accessible(
        &self,
        service: &str,
        path: &str,
    ) -> Result<AccessibleProxy<'_>, DeclareError> {
        AccessibleProxy::builder(&self.bus)
            // Qt's accessibility bridge answers Properties.Get but not GetAll, which is
            // what a cached property would need, so names are read one at a time.
            .cache_properties(zbus::proxy::CacheProperties::No)
            .destination(service.to_string())
            .and_then(|builder| builder.path(path.to_string()))
            .map_err(|e| DeclareError::Bus(e.to_string()))?
            .build()
            .await
            .map_err(|e| DeclareError::Bus(e.to_string()))
    }

    async fn bounds(&self, service: &str, path: &str) -> Option<Rect> {
        let component = ComponentProxy::builder(&self.bus)
            .destination(service.to_string())
            .ok()?
            .path(path.to_string())
            .ok()?
            .build()
            .await
            .ok()?;
        let (x, y, width, height) = component.get_extents(WINDOW_COORDS).await.ok()?;
        // A control with no size, or one the window says is outside itself, is not
        // somewhere to click.
        if x < 0 || y < 0 || width <= 0 || height <= 0 {
            return None;
        }
        Rect::new(x, y, width as u32, height as u32).ok()
    }
}
