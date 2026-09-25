//! A checked timeline, written as MLT XML.
//!
//! MLT is the engine under Kdenlive and Shotcut, and `melt` renders its XML without a
//! window. The document written here is plain MLT: a profile, one producer per source, a
//! video track of clips end to end, a track of titles laid over it, a music track mixed
//! under it, and fades on the whole.
//!
//! Every value that comes from a person or a model — a path, a title — is escaped as XML
//! text or attribute, so no text can add an element or a property of its own.

use crate::timeline::{Resolved, Timeline};
use std::fmt::Write;

/// Escapes text for an XML attribute value or element content.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// The MLT document for `timeline`, whose files were resolved by [`Timeline::validate`].
pub fn document(timeline: &Timeline, files: &Resolved) -> String {
    let fps = timeline.fps as f64;
    let frames = |seconds: f64| (seconds * fps).round() as i64;
    let total = timeline
        .clips
        .iter()
        .map(|clip| frames(clip.out_s) - frames(clip.in_s))
        .sum::<i64>()
        .max(1);
    let last = total - 1;
    let divisor = gcd(timeline.width, timeline.height).max(1);

    let mut xml = String::new();
    let _ = writeln!(xml, r#"<?xml version="1.0" encoding="utf-8"?>"#);
    let _ = writeln!(
        xml,
        r#"<mlt LC_NUMERIC="C" version="7.0.0" producer="main">"#
    );
    let _ = writeln!(
        xml,
        r#"  <profile description="archie" width="{w}" height="{h}" progressive="1" sample_aspect_num="1" sample_aspect_den="1" display_aspect_num="{dw}" display_aspect_den="{dh}" frame_rate_num="{fps}" frame_rate_den="1" colorspace="709"/>"#,
        w = timeline.width,
        h = timeline.height,
        dw = timeline.width / divisor,
        dh = timeline.height / divisor,
        fps = timeline.fps,
    );

    // Sources, then the video track: each clip cut by its in and out point, end to end.
    for (at, path) in files.clips.iter().enumerate() {
        let _ = writeln!(
            xml,
            r#"  <producer id="clip{at}"><property name="resource">{}</property></producer>"#,
            escape(&path.to_string_lossy())
        );
    }
    let _ = writeln!(xml, r#"  <playlist id="video">"#);
    for (at, clip) in timeline.clips.iter().enumerate() {
        let (from, to) = (frames(clip.in_s), frames(clip.out_s) - 1);
        let _ = writeln!(
            xml,
            r#"    <entry producer="clip{at}" in="{from}" out="{to}"/>"#
        );
    }
    let _ = writeln!(xml, "  </playlist>");

    // Titles: one text producer each, laid on their own track at their start time.
    let mut titles: Vec<_> = timeline.titles.iter().enumerate().collect();
    titles.sort_by(|a, b| a.1.start_s.total_cmp(&b.1.start_s));
    for (at, title) in &titles {
        let length = frames(title.duration_s).max(1);
        let size = (timeline.height / 14).max(12);
        let _ = writeln!(
            xml,
            r##"  <producer id="title{at}" in="0" out="{out}"><property name="mlt_service">qtext</property><property name="text">{text}</property><property name="fgcolour">#ffffffff</property><property name="olcolour">#cc000000</property><property name="outline">3</property><property name="size">{size}</property><property name="weight">700</property><property name="length">{length}</property></producer>"##,
            out = length - 1,
            text = escape(&title.text),
        );
    }
    let _ = writeln!(xml, r#"  <playlist id="titles">"#);
    let mut cursor = 0i64;
    for (at, title) in &titles {
        let start = frames(title.start_s).min(last);
        let length = frames(title.duration_s).max(1).min(total - start);
        if start > cursor {
            let _ = writeln!(xml, r#"    <blank length="{}"/>"#, start - cursor);
        }
        let _ = writeln!(
            xml,
            r#"    <entry producer="title{at}" in="0" out="{}"/>"#,
            length - 1
        );
        cursor = start + length;
    }
    if cursor < total {
        let _ = writeln!(xml, r#"    <blank length="{}"/>"#, total - cursor);
    }
    let _ = writeln!(xml, "  </playlist>");

    // Music under everything, for as long as the video runs. The producer is declared
    // outside the playlist: MLT takes a producer written inside one as an entry of its
    // own, and with the entry below the music played twice and the video ran double.
    if let (Some(music), Some(path)) = (&timeline.music, &files.music) {
        let _ = writeln!(
            xml,
            r#"  <producer id="song"><property name="resource">{}</property><filter><property name="mlt_service">volume</property><property name="gain">{:.3}</property></filter></producer>"#,
            escape(&path.to_string_lossy()),
            music.volume,
        );
    }
    let _ = writeln!(xml, r#"  <playlist id="music">"#);
    match (&timeline.music, &files.music) {
        (Some(_), Some(_)) => {
            let _ = writeln!(xml, r#"    <entry producer="song" in="0" out="{last}"/>"#);
        }
        _ => {
            let _ = writeln!(xml, r#"    <blank length="{total}"/>"#);
        }
    }
    let _ = writeln!(xml, "  </playlist>");

    let _ = writeln!(xml, r#"  <tractor id="main" in="0" out="{last}">"#);
    let _ = writeln!(xml, r#"    <track producer="video"/>"#);
    let _ = writeln!(xml, r#"    <track producer="titles"/>"#);
    let _ = writeln!(xml, r#"    <track producer="music" hide="video"/>"#);
    let _ = writeln!(
        xml,
        r#"    <transition in="0" out="{last}"><property name="mlt_service">composite</property><property name="a_track">0</property><property name="b_track">1</property><property name="geometry">0%/72%:100%x22%</property><property name="halign">centre</property><property name="valign">middle</property></transition>"#
    );
    let _ = writeln!(
        xml,
        r#"    <transition in="0" out="{last}"><property name="mlt_service">mix</property><property name="a_track">0</property><property name="b_track">2</property><property name="always_active">1</property><property name="sum">1</property></transition>"#
    );
    if timeline.fade_s > 0.0 {
        let fade = frames(timeline.fade_s).clamp(1, total / 2);
        let _ = writeln!(
            xml,
            r#"    <filter in="0" out="{last}"><property name="mlt_service">brightness</property><property name="level">0=0;{fade}=1;{hold}=1;{last}=0</property></filter>"#,
            hold = last - fade,
        );
        let _ = writeln!(
            xml,
            r#"    <filter in="0" out="{last}"><property name="mlt_service">volume</property><property name="level">0=-60;{fade}=0;{hold}=0;{last}=-60</property></filter>"#,
            hold = last - fade,
        );
    }
    let _ = writeln!(xml, "  </tractor>");
    let _ = writeln!(xml, "</mlt>");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{Clip, Music, Title};
    use std::path::PathBuf;

    fn timeline() -> (Timeline, Resolved) {
        let timeline = Timeline {
            width: 1280,
            height: 720,
            fps: 25,
            clips: vec![
                Clip {
                    source: "a.mp4".into(),
                    in_s: 1.0,
                    out_s: 3.0,
                },
                Clip {
                    source: "b.mp4".into(),
                    in_s: 0.0,
                    out_s: 2.0,
                },
            ],
            titles: vec![Title {
                text: "Archie <&> \"OS\"".into(),
                start_s: 1.0,
                duration_s: 1.0,
            }],
            music: Some(Music {
                source: "song.mp3".into(),
                volume: 0.2,
            }),
            fade_s: 0.4,
        };
        let files = Resolved {
            clips: vec![
                PathBuf::from("/media/a.mp4"),
                PathBuf::from("/media/b & c.mp4"),
            ],
            music: Some(PathBuf::from("/media/song.mp3")),
        };
        (timeline, files)
    }

    #[test]
    fn clips_run_end_to_end_in_frames_of_the_profile() {
        let (timeline, files) = timeline();
        let xml = document(&timeline, &files);
        assert!(xml.contains(r#"width="1280" height="720""#));
        assert!(xml.contains(r#"display_aspect_num="16" display_aspect_den="9""#));
        assert!(
            xml.contains(r#"<entry producer="clip0" in="25" out="74"/>"#),
            "{xml}"
        );
        assert!(xml.contains(r#"<entry producer="clip1" in="0" out="49"/>"#));
        assert!(
            xml.contains(r#"<tractor id="main" in="0" out="99">"#),
            "100 frames in all"
        );
    }

    #[test]
    fn titles_sit_at_their_time_and_text_cannot_become_markup() {
        let (timeline, files) = timeline();
        let xml = document(&timeline, &files);
        assert!(xml.contains("Archie &lt;&amp;&gt; &quot;OS&quot;"));
        assert!(!xml.contains("<&>"));
        assert!(xml.contains("/media/b &amp; c.mp4"));
        let titles = &xml[xml.find(r#"<playlist id="titles">"#).unwrap()..];
        assert!(
            titles.starts_with("<playlist id=\"titles\">\n    <blank length=\"25\"/>"),
            "{titles}"
        );
    }

    #[test]
    fn music_is_mixed_under_and_the_whole_fades() {
        let (timeline, files) = timeline();
        let xml = document(&timeline, &files);
        assert!(xml.contains(r#"<property name="gain">0.200</property>"#));
        assert!(xml.contains("<property name=\"mlt_service\">mix</property>"));
        // No producer inside a playlist: MLT would play it as an extra entry.
        for playlist in xml.split("<playlist").skip(1) {
            let body = &playlist[..playlist.find("</playlist>").unwrap()];
            assert!(
                !body.contains("<producer"),
                "a producer inside a playlist: {body}"
            );
        }
        assert!(xml.contains("0=0;10=1;89=1;99=0"), "{xml}");

        let mut silent = timeline.clone();
        silent.music = None;
        silent.fade_s = 0.0;
        let xml = document(
            &silent,
            &Resolved {
                clips: files.clips.clone(),
                music: None,
            },
        );
        assert!(
            xml.contains(r#"<playlist id="music">"#) && xml.contains(r#"<blank length="100"/>"#)
        );
        assert!(!xml.contains("brightness"));
    }
}
