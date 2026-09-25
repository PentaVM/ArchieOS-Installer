//! Archie edits video.
//!
//! An agent — Archie's voice, or an IDE over MCP — describes a video as a [`Timeline`]:
//! clips cut from sources, titles over them, music under them, a fade at each end. The
//! timeline is checked and every file in it confined to one media folder
//! ([`timeline`]), written as MLT XML ([`mlt`]), and rendered by `melt` ([`render`]), the
//! engine under Kdenlive and Shotcut. The project file stays beside the video, so a person
//! can open it in Kdenlive and keep editing by hand.
//!
//! No Python and no scripting host: the engines are C and C++, run as programs with
//! arguments built here.

pub mod mlt;
pub mod render;
pub mod timeline;

pub use render::{probe, render, Probe, RenderError, Rendered, RENDERS};
pub use timeline::{Clip, Music, Timeline, TimelineError, Title};
