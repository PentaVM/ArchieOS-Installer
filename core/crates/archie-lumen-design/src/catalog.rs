//! Typed catalog for XaviCottier's visual-presets/catalog.js. Numeric discriminants
//! are part of the C ABI. The JSON catalog retains every source preset value.
macro_rules! catalog_enum {
    ($name:ident { $($variant:ident = $id:literal => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u32)]
        pub enum $name { $($variant = $id),+ }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn from_id(id: u32) -> Option<Self> { match id { $($id => Some(Self::$variant),)+ _ => None } }
            pub fn parse(text: &str) -> Option<Self> { match text { $($text => Some(Self::$variant),)+ _ => None } }
            pub const fn as_str(self) -> &'static str { match self { $(Self::$variant => $text),+ } }
        }
    };
}
catalog_enum!(NodeStyle { Original=0=>"astra_original", PrismCut=1=>"prism_cut", Obsidian=2=>"obsidian", BrushedTitanium=3=>"brushed_titanium", Hologram=4=>"hologram", FracturedCrystal=5=>"fractured_crystal", Microcircuit=6=>"microcircuit", PhotonSkin=7=>"photon_skin", MatteCeramic=8=>"matte_ceramic" });
catalog_enum!(LinkStyle { Original=0=>"astra_original", HairlineDual=1=>"hairline_dual", FiberTwist=2=>"fiber_twist", Technical=3=>"technical", DashPhase=4=>"dash_phase", RefractedThread=5=>"refracted_thread", CircuitDash=6=>"circuit_dash", PlasmaRibbon=7=>"plasma_ribbon", Hairline=8=>"hairline" });
catalog_enum!(PulseStyle { Original=0=>"astra_original", SoftPacket=1=>"soft_packet", TravelingGlint=2=>"traveling_glint", SubtleTick=3=>"subtle_tick", ScanPacket=4=>"scan_packet", FacetSpark=5=>"facet_spark", DataPacket=6=>"data_packet", PlasmaBead=7=>"plasma_bead", None=8=>"none" });
catalog_enum!(BloomMode { Original=0=>"astra_original", Global=1=>"global", Selection=2=>"selection", Polar=3=>"polar", Off=4=>"off" });
catalog_enum!(BloomMask { Bright=0=>"bright", SelectedAndNeighbors=1=>"selected_and_neighbors", PolesAndFocus=2=>"poles_and_focus", None=3=>"none" });
catalog_enum!(TextureTier { Adaptive=0=>"adaptive", Procedural=1=>"procedural" });

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualSettings {
    pub node_style: NodeStyle,
    pub folder_style: NodeStyle,
    pub link_style: LinkStyle,
    pub pulse_style: PulseStyle,
    pub bloom_mode: BloomMode,
    /// Preserved source metadata. Source rendering derives its mask from bloom_mode.
    pub bloom_mask: BloomMask,
    pub texture_tier: TextureTier,
    pub pole_color_mix: f32,
    pub glow_mask_intensity: f32,
    pub line_width_px: f32,
    pub detail_frequency: f32,
}
impl Default for VisualSettings {
    fn default() -> Self {
        Self {
            node_style: NodeStyle::Original,
            folder_style: NodeStyle::Original,
            link_style: LinkStyle::Original,
            pulse_style: PulseStyle::Original,
            bloom_mode: BloomMode::Original,
            bloom_mask: BloomMask::Bright,
            texture_tier: TextureTier::Adaptive,
            pole_color_mix: 0.,
            glow_mask_intensity: 1.,
            line_width_px: 1.,
            detail_frequency: 1.,
        }
    }
}
impl VisualSettings {
    pub fn validated(mut self) -> Self {
        let d = Self::default();
        self.pole_color_mix = bounded(self.pole_color_mix, 0., 1., d.pole_color_mix);
        self.glow_mask_intensity = bounded(self.glow_mask_intensity, 0., 2., d.glow_mask_intensity);
        self.line_width_px = bounded(self.line_width_px, 0.5, 4., d.line_width_px);
        self.detail_frequency = bounded(self.detail_frequency, 0.1, 3., d.detail_frequency);
        self
    }
}
fn bounded(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Structure,
    Links,
    Material,
    Uniform,
}
/// Same precedence as policy.js. The caller supplies keys whose values changed.
pub fn change_kind<'a>(keys: impl IntoIterator<Item = &'a str>) -> ChangeKind {
    let mut links = false;
    let mut material = false;
    for key in keys {
        if [
            "layoutMode",
            "shellStyle",
            "shellDensity",
            "shellScale",
            "ambientCount",
            "linkBudget",
            "highlightBudget",
            "guides",
        ]
        .contains(&key)
        {
            return ChangeKind::Structure;
        }
        links |= key == "visualLinkStyle";
        material |= [
            "visualNodeStyle",
            "visualFolderStyle",
            "visualTextureTier",
            "textureResolution",
            "textureContrast",
            "visualDetailFrequency",
        ]
        .contains(&key);
    }
    if links {
        ChangeKind::Links
    } else if material {
        ChangeKind::Material
    } else {
        ChangeKind::Uniform
    }
}
pub const CATALOG_JSON: &str = include_str!("catalog.json");
pub(crate) static CATALOG_C: &str = concat!(include_str!("catalog.json"), "\0");
