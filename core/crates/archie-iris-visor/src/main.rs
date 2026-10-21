//! archie-iris-visor: what Archie sees, in an ArchieOS window.
//!
//! A camera's frames arrive over loopback from `bridge/camera_bridge.py` (it runs where the
//! camera is: under WSL that is Windows, since the VM sees no USB camera). Every frame goes
//! to IRIS's live endpoint, which answers each one from damage and tracking and reads with
//! the model when the scene changes; boxes IRIS finds text in are read by IRIS's reading head
//! and what is new goes to Wanda (`POST /v1/scene/text`), whose LNCM updates its social state
//! in place. Everything stays on loopback.
//!
//! The window follows the shell's `AppWindow.qml`: a 52 px title bar with traffic lights and a
//! centred title, a 200 px sidebar, the main area, the dark theme's tokens
//! (`display/shell/prototype/Theme.qml`) and its fonts, Space Grotesk and Inter.
//!
//! ```text
//! archie-iris-visor [--camera 127.0.0.1:7671] [--iris http://127.0.0.1:8765]
//!     [--wanda http://127.0.0.1:8791 --wanda-token-file <owner.token> --domain personal/owner]
//!     [--phrases person,face,hand,cup,phone,screen,text,keyboard,book,bottle] [--threshold 0.35]
//!     [--ocr-url http://127.0.0.1:8792]   (a local vision-language model reads the text boxes)
//!     [--ocr-expert]   (IRIS's own OCR expert, GOT-OCR 2.0 behind `POST /ocr`, reads the frame)
//! ```

mod state;
mod workers;

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Rect,
    Rounding, Sense, Stroke, TextureHandle, TextureOptions,
};
use state::{Shared, State};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use workers::Settings;

// Theme.qml, dark, blurred off (WSLg composes no blur): the opaque values.
const DESKTOP: Color32 = Color32::from_rgb(0x07, 0x08, 0x0c);
const WIN: Color32 = Color32::from_rgb(0x12, 0x13, 0x1a);
const SIDE: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x24);
const PANEL: Color32 = Color32::from_rgb(0x16, 0x17, 0x1f);
const HAIR: Color32 = Color32::from_rgb(0x2c, 0x2d, 0x36);
const INNER: Color32 = Color32::from_rgb(0x25, 0x26, 0x2e);
const T1: Color32 = Color32::from_rgb(0xf2, 0xf1, 0xee);
const T2: Color32 = Color32::from_rgb(0xb9, 0xb8, 0xc4);
const T3: Color32 = Color32::from_rgb(0x8d, 0x8c, 0x9c);
const ACC: Color32 = Color32::from_rgb(0x6f, 0xc8, 0xf5);
const ARCH: Color32 = Color32::from_rgb(0xf0, 0xbb, 0x78);
const OK: Color32 = Color32::from_rgb(0x47, 0xd1, 0x8b);
const WARN: Color32 = Color32::from_rgb(0xf0, 0xb4, 0x29);
const BAD: Color32 = Color32::from_rgb(0xf2, 0x62, 0x6b);
const INFO: Color32 = Color32::from_rgb(0x6f, 0xe3, 0xf5);
const TL_CLOSE: Color32 = Color32::from_rgb(0xd9, 0x60, 0x5a);
const TL_MIN: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);
const TL_ZOOM: Color32 = Color32::from_rgb(0x4f, 0xb6, 0x7b);
const R_WINDOW: f32 = 14.0;
const R_CARD: f32 = 10.0;
const TITLE_H: f32 = 52.0;
const SIDE_W: f32 = 200.0;

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        FontData::from_static(include_bytes!(
            "../../../../display/shell/fonts/inter/Inter-Variable.ttf"
        )),
    );
    fonts.font_data.insert(
        "grotesk".into(),
        FontData::from_static(include_bytes!(
            "../../../../display/shell/fonts/space-grotesk/SpaceGrotesk-Variable.ttf"
        )),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".into());
    fonts.families.insert(
        FontFamily::Name("grotesk".into()),
        vec!["grotesk".into(), "inter".into()],
    );
    fonts
}

fn disp(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("grotesk".into()))
}

fn text(size: f32) -> FontId {
    FontId::proportional(size)
}

/// A state: a dot and a word, never colour alone (Theme.qml's rule).
fn chip(ui: &mut egui::Ui, colour: Color32, word: &str) {
    let galley = ui.painter().layout_no_wrap(word.to_owned(), text(12.5), T1);
    let size = vec2(galley.size().x + 28.0, 24.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(12.0), INNER);
    p.rect_stroke(rect, Rounding::same(12.0), Stroke::new(1.0, HAIR));
    p.circle_filled(pos2(rect.left() + 12.0, rect.center().y), 4.0, colour);
    p.galley(
        pos2(rect.left() + 21.0, rect.center().y - galley.size().y / 2.0),
        galley,
        T1,
    );
}

fn eyebrow(ui: &mut egui::Ui, label: &str) {
    ui.add_space(10.0);
    ui.label(
        egui::RichText::new(label.to_uppercase())
            .font(text(11.0))
            .color(T3),
    );
    ui.add_space(2.0);
}

fn row(ui: &mut egui::Ui, key: &str, value: String, colour: Color32) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(key).font(text(13.0)).color(T2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).font(text(13.0)).color(colour));
        });
    });
}

fn phrase_colour(phrase: &str) -> Color32 {
    const PALETTE: [Color32; 6] = [
        ACC,
        ARCH,
        OK,
        INFO,
        WARN,
        Color32::from_rgb(0xbe, 0x9b, 0xf5),
    ];
    let hash = phrase
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    PALETTE[(hash as usize) % PALETTE.len()]
}

struct Visor {
    state: State,
    texture: Option<TextureHandle>,
    shown: u64,
    labels: bool,
    settings: Arc<Settings>,
}

impl Visor {
    fn title_bar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), TITLE_H));
        let response = ui.interact(bar, ui.id().with("title"), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if response.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        let p = ui.painter();
        p.line_segment(
            [
                pos2(bar.left(), bar.bottom()),
                pos2(bar.right(), bar.bottom()),
            ],
            Stroke::new(1.0, HAIR),
        );
        // Traffic lights: close, minimise, zoom.
        let lights = [
            (TL_CLOSE, egui::ViewportCommand::Close),
            (TL_MIN, egui::ViewportCommand::Minimized(true)),
            (
                TL_ZOOM,
                egui::ViewportCommand::Maximized(
                    !ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false)),
                ),
            ),
        ];
        for (index, (colour, command)) in lights.into_iter().enumerate() {
            let centre = pos2(bar.left() + 22.0 + index as f32 * 20.0, bar.center().y);
            let hit = Rect::from_center_size(centre, vec2(14.0, 14.0));
            let light = ui.interact(hit, ui.id().with(("light", index)), Sense::click());
            let p = ui.painter();
            p.circle_filled(centre, 6.0, colour);
            p.circle_stroke(centre, 6.0, Stroke::new(1.0, Color32::from_black_alpha(56)));
            if light.clicked() {
                ui.ctx().send_viewport_cmd(command);
            }
        }
        ui.painter().text(
            bar.center(),
            Align2::CENTER_CENTER,
            "IRIS · Lo que ve Archie",
            disp(17.0),
            T1,
        );
        // Trailing: IRIS and Wanda, each a state with its word.
        let (iris_on, wanda_on) = self
            .state
            .lock()
            .map(|s| (s.iris.online, s.wanda.online))
            .unwrap_or_default();
        let mut trail = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(bar.right() - 260.0, bar.top()),
                    bar.max,
                ))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        trail.add_space(16.0);
        chip(
            &mut trail,
            if wanda_on { OK } else { BAD },
            if wanda_on {
                "Wanda en línea"
            } else {
                "Wanda sin conexión"
            },
        );
        trail.add_space(8.0);
        chip(
            &mut trail,
            if iris_on { OK } else { BAD },
            if iris_on {
                "IRIS en línea"
            } else {
                "IRIS sin conexión"
            },
        );
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        ui.painter().rect_filled(
            rect,
            Rounding {
                sw: R_WINDOW,
                ..Rounding::ZERO
            },
            SIDE,
        );
        ui.painter().line_segment(
            [rect.right_top(), rect.right_bottom()],
            Stroke::new(1.0, HAIR),
        );
        let mut side = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(14.0, 10.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let Ok(mut s) = self.state.lock() else {
            return;
        };
        let ui = &mut side;
        eyebrow(ui, "Cámara");
        row(
            ui,
            "Elgato",
            if s.camera_connected {
                "conectada".into()
            } else {
                "esperando".into()
            },
            if s.camera_connected { OK } else { WARN },
        );
        row(ui, "Cuadros/s", format!("{:.0}", s.camera.per_second()), T1);
        if let Some(f) = &s.frame {
            row(ui, "Tamaño", format!("{}×{}", f.width, f.height), T2);
        }
        eyebrow(ui, "IRIS");
        row(
            ui,
            "Respondidos/s",
            format!("{:.0}", s.iris.answers.per_second()),
            T1,
        );
        row(
            ui,
            "Lecturas/s",
            format!("{:.1}", s.iris.reads.per_second()),
            ACC,
        );
        row(ui, "Respuesta", format!("{:.1} ms", s.iris.answer_ms), T1);
        row(
            ui,
            "Última lectura",
            s.iris
                .inference_age_ms
                .map_or("—".into(), |a| format!("hace {a} ms")),
            T2,
        );
        let decision = match s.iris.decision.as_str() {
            "infer" if s.iris.open_sweep => ("lee todo", ARCH),
            "infer" => ("lee", ACC),
            "track" => ("sigue", INFO),
            "reuse" => ("sin cambios", T2),
            "idle" => ("en reposo", T3),
            _ => ("—", T3),
        };
        row(ui, "Decisión", decision.0.into(), decision.1);
        row(ui, "Cambio", format!("{:.0} %", s.iris.damage * 100.0), T2);
        eyebrow(ui, "Wanda");
        row(ui, "Llamadas OCR", s.wanda.calls.to_string(), T1);
        row(ui, "Aplicados", s.wanda.applied.to_string(), OK);
        row(ui, "Sin cambio", s.wanda.unchanged.to_string(), T2);
        row(ui, "Rechazados", s.wanda.rejected.to_string(), T2);
        eyebrow(ui, "Opciones");
        ui.checkbox(&mut self.labels, egui::RichText::new("Etiquetas").color(T1));
        ui.checkbox(
            &mut s.ocr_enabled,
            egui::RichText::new("Leer texto → Wanda").color(T1),
        );
        if let Some(error) = s.iris.error.clone().or(s.wanda.error.clone()) {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(error).font(text(11.5)).color(BAD));
        }
    }

    fn camera_view(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(rect, Rounding::same(R_CARD), DESKTOP);
        p.rect_stroke(rect, Rounding::same(R_CARD), Stroke::new(1.0, HAIR));
        let Ok(s) = self.state.lock() else {
            return;
        };
        let Some(frame) = &s.frame else {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Esperando la cámara…\nbridge/camera_bridge.py en Windows",
                text(15.0),
                T3,
            );
            return;
        };
        if frame.id != self.shown {
            let image = egui::ColorImage::from_rgb([frame.width, frame.height], &frame.rgb);
            match &mut self.texture {
                Some(texture) => texture.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        "camara",
                        image,
                        TextureOptions::LINEAR,
                    ))
                }
            }
            self.shown = frame.id;
        }
        let (fw, fh) = (frame.width as f32, frame.height as f32);
        let scale = (rect.width() / fw).min(rect.height() / fh);
        let shown = Rect::from_center_size(rect.center(), vec2(fw * scale, fh * scale));
        if let Some(texture) = &self.texture {
            p.image(
                texture.id(),
                shown,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        // Where the OCR's own attention found text (for a few seconds after it looked): a
        // dashed white frame and what it read underneath.
        if s.spots_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(6))
        {
            // A reader that does not locate (the OCR expert) gives no box to draw.
            for spot in s.spots.iter().filter(|spot| spot.bbox[2] > spot.bbox[0]) {
                let b = Rect::from_min_max(
                    pos2(
                        shown.left() + spot.bbox[0] * scale,
                        shown.top() + spot.bbox[1] * scale,
                    ),
                    pos2(
                        shown.left() + spot.bbox[2] * scale,
                        shown.top() + spot.bbox[3] * scale,
                    ),
                );
                let stroke = Stroke::new(1.5, T1);
                for (a, z) in [
                    (b.left_top(), b.right_top()),
                    (b.right_top(), b.right_bottom()),
                    (b.right_bottom(), b.left_bottom()),
                    (b.left_bottom(), b.left_top()),
                ] {
                    p.extend(egui::Shape::dashed_line(&[a, z], stroke, 6.0, 4.0));
                }
                let galley = p.layout_no_wrap(
                    format!("texto: {}", spot.text),
                    text(12.5),
                    Color32::from_rgb(4, 18, 28),
                );
                let tag = Rect::from_min_size(
                    pos2(b.left(), (b.bottom() + 2.0).min(shown.bottom() - 20.0)),
                    vec2(galley.size().x + 10.0, 20.0),
                );
                p.rect_filled(tag, Rounding::same(4.0), T1);
                p.galley(
                    pos2(tag.left() + 5.0, tag.top() + 2.0),
                    galley,
                    Color32::BLACK,
                );
            }
        }
        for seen in &s.iris.seen {
            let colour = phrase_colour(&seen.phrase);
            let b = Rect::from_min_max(
                pos2(
                    shown.left() + seen.bbox[0] * scale,
                    shown.top() + seen.bbox[1] * scale,
                ),
                pos2(
                    shown.left() + seen.bbox[2] * scale,
                    shown.top() + seen.bbox[3] * scale,
                ),
            );
            let fresh = seen.source == "inferred";
            let stroke = Stroke::new(
                if fresh { 2.5 } else { 1.5 },
                if fresh {
                    colour
                } else {
                    colour.gamma_multiply(0.75)
                },
            );
            p.rect_stroke(b, Rounding::same(4.0), stroke);
            if self.labels {
                let how = match seen.source.as_str() {
                    "inferred" => "leído",
                    "tracked" => "seguido",
                    "reused" => "igual",
                    _ => "perdido",
                };
                let label = format!("{} {:.0}% · {how}", seen.phrase, seen.score * 100.0);
                let galley = p.layout_no_wrap(label, text(12.5), Color32::from_rgb(4, 18, 28));
                let tag = Rect::from_min_size(
                    pos2(b.left(), (b.top() - 20.0).max(shown.top())),
                    vec2(galley.size().x + 10.0, 20.0),
                );
                p.rect_filled(tag, Rounding::same(4.0), colour);
                p.galley(
                    pos2(tag.left() + 5.0, tag.top() + 2.0),
                    galley,
                    Color32::BLACK,
                );
            }
        }
    }

    fn feed(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter();
        p.rect_filled(rect, Rounding::same(R_CARD), PANEL);
        p.rect_stroke(rect, Rounding::same(R_CARD), Stroke::new(1.0, HAIR));
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink(12.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut inner;
        let Ok(s) = self.state.lock() else {
            return;
        };
        ui.label(egui::RichText::new("Lo que ve").font(disp(15.0)).color(T1));
        ui.add_space(4.0);
        let mut seen = s.iris.seen.clone();
        seen.sort_by(|a, b| b.score.total_cmp(&a.score));
        egui::ScrollArea::vertical()
            .id_salt("seen")
            .max_height(rect.height() * 0.38)
            .show(ui, |ui| {
                if seen.is_empty() {
                    ui.label(egui::RichText::new("nada sobre el umbral").color(T3));
                }
                for d in &seen {
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                        ui.painter()
                            .circle_filled(dot.center(), 4.0, phrase_colour(&d.phrase));
                        ui.label(egui::RichText::new(&d.phrase).color(T1));
                        ui.label(
                            egui::RichText::new(format!(
                                "{:.0}% · #{} · {}",
                                d.score * 100.0,
                                d.track,
                                d.source
                            ))
                            .font(text(12.0))
                            .color(T3),
                        );
                    });
                }
            });
        ui.add_space(8.0);
        ui.separator();
        ui.label(
            egui::RichText::new("OCR → Wanda")
                .font(disp(15.0))
                .color(T1),
        );
        ui.label(
            egui::RichText::new(format!(
                "{} cada {:.1} s · Wanda {}",
                if self.settings.ocr_expert {
                    "El experto OCR de IRIS (GOT-OCR 2.0) lee el cuadro entero"
                } else if self.settings.ocr_url.is_some() {
                    "Qwen3-VL-4B busca y lee el texto del cuadro entero"
                } else {
                    "IRIS read_decoder lee las cajas de texto de IRIS"
                },
                self.settings.ocr_every.as_secs_f32(),
                s.wanda.domain
            ))
            .font(text(11.5))
            .color(T3),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("ocr").show(ui, |ui| {
            if s.ocr.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "Todavía no hubo texto: IRIS busca cajas «text», «sign», «screen»…",
                    )
                    .color(T3),
                );
            }
            for call in &s.ocr {
                let age = call.at.elapsed().as_secs();
                ui.label(
                    egui::RichText::new(format!(
                        "hace {age} s · {} · {} cajas · {:.0} ms · {} nuevas",
                        call.reader, call.boxes, call.read_ms, call.sent
                    ))
                    .font(text(11.5))
                    .color(T3),
                );
                for line in &call.lines {
                    let shown = if line.is_empty() {
                        "(vacío)"
                    } else {
                        line.as_str()
                    };
                    ui.label(egui::RichText::new(format!("“{shown}”")).color(T1));
                }
                match &call.wanda {
                    Ok(text) => ui.label(egui::RichText::new(format!("Wanda: {text}")).color(OK)),
                    Err(error) => {
                        ui.label(egui::RichText::new(format!("Wanda: {error}")).color(BAD))
                    }
                };
                ui.add_space(6.0);
            }
        });
    }
}

impl eframe::App for Visor {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(DESKTOP).to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(DESKTOP))
            .show(ctx, |ui| {
                let window = ui.max_rect();
                let p = ui.painter();
                p.rect_filled(window, Rounding::same(R_WINDOW), WIN);
                p.rect_stroke(window, Rounding::same(R_WINDOW), Stroke::new(1.0, HAIR));
                self.title_bar(ui, window);
                let body =
                    Rect::from_min_max(pos2(window.left(), window.top() + TITLE_H), window.max);
                let side = Rect::from_min_size(body.min, vec2(SIDE_W, body.height()));
                self.sidebar(ui, side);
                let main = Rect::from_min_max(
                    pos2(side.right() + 20.0, body.top() + 18.0),
                    body.max - vec2(20.0, 18.0),
                );
                let feed_w = 340.0f32.min(main.width() * 0.35);
                let camera =
                    Rect::from_min_max(main.min, pos2(main.right() - feed_w - 16.0, main.bottom()));
                let feed = Rect::from_min_max(pos2(camera.right() + 16.0, main.top()), main.max);
                self.camera_view(ui, camera);
                self.feed(ui, feed);
            });
        ctx.request_repaint_after(Duration::from_millis(8));
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let token = arg(&args, "--wanda-token-file")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
    let ocr_expert = args.iter().any(|a| a == "--ocr-expert");
    let attention = (ocr_expert || arg(&args, "--ocr-url").is_some())
        && !args.iter().any(|a| a == "--ocr-on-iris-boxes");
    let settings = Arc::new(Settings {
        camera_listen: arg(&args, "--camera").unwrap_or_else(|| "127.0.0.1:7671".into()),
        iris: arg(&args, "--iris").unwrap_or_else(|| "http://127.0.0.1:8765".into()),
        phrases: arg(&args, "--phrases")
            .unwrap_or_else(|| "person,face,hand,cup,phone,screen,text,keyboard,book,bottle".into())
            .replace(' ', "%20"),
        threshold: arg(&args, "--threshold")
            .and_then(|t| t.parse().ok())
            .unwrap_or(0.35),
        wanda: arg(&args, "--wanda").unwrap_or_else(|| "http://127.0.0.1:8791".into()),
        wanda_token: token,
        domain: arg(&args, "--domain").unwrap_or_else(|| "personal/owner".into()),
        // A whole-frame look takes the model a few seconds; a crop well under one.
        ocr_every: Duration::from_millis(
            arg(&args, "--ocr-ms")
                .and_then(|t| t.parse().ok())
                .unwrap_or(if ocr_expert {
                    1000
                } else if attention {
                    3000
                } else {
                    1500
                }),
        ),
        ocr_url: arg(&args, "--ocr-url"),
        ocr_expert,
    });
    let state: State = Arc::new(Mutex::new(Shared::default()));
    {
        let (s, listen) = (state.clone(), settings.camera_listen.clone());
        std::thread::spawn(move || workers::camera(s, listen));
    }
    // With a model, the OCR looks for text itself; `--ocr-on-iris-boxes` reads only the
    // boxes IRIS labelled as text (the only way with IRIS's own reading head).
    let reader: fn(State, Arc<Settings>) = if attention {
        workers::ocr_attention
    } else {
        workers::ocr
    };
    let jobs: [fn(State, Arc<Settings>); 3] = [workers::iris, reader, workers::wanda_health];
    for work in jobs {
        let (s, settings) = (state.clone(), settings.clone());
        std::thread::spawn(move || work(s, settings));
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("IRIS · Lo que ve Archie")
            .with_app_id("archie-iris-visor")
            .with_inner_size([1480.0, 900.0])
            .with_min_inner_size([960.0, 600.0])
            .with_decorations(false),
        ..Default::default()
    };
    eframe::run_native(
        "archie-iris-visor",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(fonts());
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = WIN;
            visuals.override_text_color = Some(T1);
            visuals.selection.bg_fill = ACC.gamma_multiply(0.35);
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(Visor {
                state,
                texture: None,
                shown: 0,
                labels: true,
                settings,
            }))
        }),
    )
}
