//! The Purple window: pick a skin per champion.
//!
//! It is only a view. The engine runs in the resident process; this window writes the choices to
//! `config.json` and reads what the engine is doing from `status.json`.

use crate::catalog::{Catalog, Skin};
use crate::config::Config;
use crate::icon;
use crate::images::Images;
use crate::lcu::Lcu;
use crate::status::Status;
use crate::win;
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Margin, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, Vec2,
};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Neutral greys; the accent marks the current choice and nothing else.
mod color {
    use egui::Color32;
    pub const WINDOW: Color32 = Color32::from_rgb(17, 17, 19);
    pub const SIDEBAR: Color32 = Color32::from_rgb(22, 22, 24);
    pub const RAISED: Color32 = Color32::from_rgb(30, 30, 33);
    pub const HOVER: Color32 = Color32::from_rgb(37, 37, 41);
    pub const BORDER: Color32 = Color32::from_rgb(42, 42, 46);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(64, 64, 70);
    pub const TEXT: Color32 = Color32::from_rgb(237, 237, 239);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(139, 139, 147);
    pub const TEXT_FAINT: Color32 = Color32::from_rgb(98, 98, 106);
    pub const ACCENT: Color32 = Color32::from_rgb(139, 92, 246);
    pub const SUCCESS: Color32 = Color32::from_rgb(52, 199, 89);
    pub const WARNING: Color32 = Color32::from_rgb(255, 176, 32);
    pub const DANGER: Color32 = Color32::from_rgb(255, 92, 92);
}

const SIDEBAR_WIDTH: f32 = 260.0;
const ROW_HEIGHT: f32 = 44.0;
const PORTRAIT: f32 = 28.0;
const CARD_MIN_WIDTH: f32 = 164.0;
const CARD_GAP: f32 = 16.0;
/// Height of the name under a card's (square) picture.
const CARD_LABEL: f32 = 40.0;
const RADIUS: u8 = 6;
const POLL: Duration = Duration::from_millis(500);
const SEMIBOLD: &str = "semibold";

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Which champions the list shows.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Filter {
    All,
    Chosen,
}

struct App {
    ctx: egui::Context,
    catalog_updates: Receiver<Catalog>,
    catalog_sender: Sender<Catalog>,
    catalog: Catalog,
    config: Config,
    images: Images,
    logo: TextureHandle,
    selected: Option<u32>,
    search: String,
    filter: Filter,
    status: Status,
    engine_running: bool,
    last_poll: Option<Instant>,
    error: Option<String>,
    screenshot: Option<Screenshot>,
    /// Scroll the list and the grid to the current selection.
    reveal: bool,
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(win::WINDOW_TITLE)
            .with_inner_size([1080.0, 700.0])
            .with_min_inner_size([820.0, 520.0])
            .with_icon(egui::IconData { rgba: icon::rgba(64), width: 64, height: 64 }),
        ..Default::default()
    };
    eframe::run_native(win::WINDOW_TITLE, options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

/// Segoe UI (shipped with Windows) when available, egui's built-in font otherwise.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let read = |name: &str| std::fs::read(format!(r"C:\Windows\Fonts\{name}")).ok();
    let mut proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = read("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(egui::FontData::from_owned(bytes)));
        proportional.insert(0, "segoe".into());
    }
    let mut strong = proportional.clone();
    if let Some(bytes) = read("seguisb.ttf") {
        fonts.font_data.insert("segoe-semibold".into(), Arc::new(egui::FontData::from_owned(bytes)));
        strong.insert(0, "segoe-semibold".into());
    }
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), strong);
    ctx.set_fonts(fonts);
}

fn install_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = color::WINDOW;
    visuals.window_fill = color::WINDOW;
    visuals.extreme_bg_color = color::RAISED;
    visuals.override_text_color = Some(color::TEXT);
    visuals.selection.bg_fill = color::ACCENT.gamma_multiply(0.6);
    visuals.selection.stroke = Stroke::new(1.0, color::TEXT);
    visuals.text_cursor.stroke = Stroke::new(1.5, color::TEXT);
    for (widget, fill, stroke) in [
        (&mut visuals.widgets.inactive, color::RAISED, color::BORDER),
        (&mut visuals.widgets.hovered, color::HOVER, color::BORDER_STRONG),
        (&mut visuals.widgets.active, color::HOVER, color::BORDER_STRONG),
        (&mut visuals.widgets.open, color::HOVER, color::BORDER_STRONG),
        (&mut visuals.widgets.noninteractive, color::WINDOW, color::BORDER),
    ] {
        widget.corner_radius = CornerRadius::same(RADIUS);
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, stroke);
        widget.expansion = 0.0;
    }
    ctx.set_visuals(visuals);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(10.0, 4.0);
        style.spacing.scroll.floating = true;
        style.text_styles.insert(egui::TextStyle::Body, regular(13.5));
        style.text_styles.insert(egui::TextStyle::Button, regular(13.0));
    });
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        install_fonts(ctx);
        install_style(ctx);
        // Dark title bar, matching the window.
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));
        let (catalog_sender, catalog_updates) = mpsc::channel();
        let catalog = Catalog::load_cached().unwrap_or_default();
        let config = Config::load().unwrap_or_default();
        let screenshot = Screenshot::from_env();
        let selected = screenshot
            .as_ref()
            .and_then(|s| s.champion.as_deref())
            .and_then(|name| catalog.champion(name))
            .map(|c| c.id)
            .or_else(|| config.skins.keys().next().copied())
            .or_else(|| catalog.champions.first().map(|c| c.id));
        let logo = ctx.load_texture(
            "purple-logo",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &icon::rgba(64)),
            egui::TextureOptions::LINEAR,
        );
        let app = Self {
            ctx: ctx.clone(),
            catalog_updates,
            catalog_sender,
            catalog,
            config,
            images: Images::new(ctx),
            logo,
            selected,
            search: String::new(),
            filter: Filter::All,
            status: Status::default(),
            engine_running: false,
            last_poll: None,
            error: None,
            screenshot,
            reveal: true,
        };
        app.refresh_catalog();
        app
    }

    fn skin_name(&self, id: u32) -> String {
        self.catalog.skins.iter().find(|s| s.id == id).map_or_else(|| format!("skin {id}"), |s| s.name.clone())
    }

    /// Reloads the catalog in the background, from the client's files (or the running client).
    fn refresh_catalog(&self) {
        let (sender, ctx) = (self.catalog_sender.clone(), self.ctx.clone());
        thread::spawn(move || {
            let loaded = Catalog::load_from_files().or_else(|_| Catalog::load(&Lcu::connect()?));
            if let Ok(catalog) = loaded {
                let _ = catalog.save();
                if sender.send(catalog).is_ok() {
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Reads the engine's status twice a second while the window is open.
    fn poll_engine(&mut self) {
        if self.last_poll.is_some_and(|t| t.elapsed() < POLL) {
            return;
        }
        self.last_poll = Some(Instant::now());
        let was_up = self.status.client_up;
        self.status = Status::read().unwrap_or_default();
        self.engine_running = win::is_running(win::DAEMON_MUTEX);
        if !self.engine_running {
            self.status = Status::default();
        }
        if self.status.client_up && !was_up {
            self.refresh_catalog();
        }
    }

    /// One line on what the engine is doing, and the colour of its dot.
    fn engine_state(&self) -> (String, Color32) {
        if !self.engine_running {
            return ("Not running".into(), color::DANGER);
        }
        if !self.status.client_up {
            return ("Waiting for the League client".into(), color::TEXT_FAINT);
        }
        if let Some(armed) = &self.status.armed {
            let verb = if self.status.in_game { "In game" } else { "Ready" };
            return (format!("{verb} · {}", self.skin_name(armed.skin)), color::SUCCESS);
        }
        let text = match self.status.phase.as_str() {
            "ChampSelect" => "Champion select",
            "GameStart" | "InProgress" | "Reconnect" => "In game",
            "Lobby" | "Matchmaking" | "ReadyCheck" => "In lobby",
            _ => "Connected to the client",
        };
        (text.into(), color::SUCCESS)
    }

    /// A problem worth a notice, if any: (message, colour).
    fn problem(&self) -> Option<(String, Color32)> {
        if self.status.patcher_outdated {
            return Some((
                "LTK Patcher does not support this game version yet. Purple updates it before your next game.".into(),
                color::WARNING,
            ));
        }
        if self.status.overlay_unreadable {
            return Some(("The game could not read the skin files. No skin will show this game.".into(), color::WARNING));
        }
        self.error.as_ref().or(self.status.error.as_ref()).map(|e| (e.clone(), color::DANGER))
    }

    fn choose(&mut self, champion: u32, skin: Option<u32>) {
        match skin {
            Some(skin) => self.config.skins.insert(champion, skin),
            None => self.config.skins.remove(&champion),
        };
        self.error = match self.config.save() {
            Ok(()) => None,
            Err(e) => Some(format!("Could not save the choice: {e:#}")),
        };
    }

    // ---- Sidebar ------------------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add(egui::Image::from_texture(&self.logo).fit_to_exact_size(Vec2::splat(22.0)));
            ui.label(egui::RichText::new("Purple").font(semibold(16.0)));
        });
        ui.add_space(12.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text(egui::RichText::new("Search").color(color::TEXT_FAINT))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        ui.add_space(2.0);
        let counts = (self.catalog.champions.len(), self.config.skins.len());
        segmented(ui, &mut self.filter, [(Filter::All, "All", counts.0), (Filter::Chosen, "With a skin", counts.1)]);
        ui.add_space(4.0);

        // The engine's state sits at the bottom of the sidebar.
        let footer = 44.0;
        let list_height = (ui.available_height() - footer).max(0.0);
        ui.allocate_ui(Vec2::new(ui.available_width(), list_height), |ui| self.champion_list(ui));
        self.footer(ui);
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        ui.painter().hline(rect.x_range().expand(14.0), rect.top(), Stroke::new(1.0, color::BORDER));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let (text, dot) = self.engine_state();
            let (dot_rect, _) = ui.allocate_exact_size(Vec2::new(8.0, 16.0), Sense::hover());
            ui.painter().circle_filled(dot_rect.center(), 3.5, dot);
            ui.add(egui::Label::new(egui::RichText::new(text).color(color::TEXT_DIM).size(12.5)).truncate());
            if !self.engine_running {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Start").clicked() {
                        self.error = win::spawn_daemon().err().map(|e| format!("Could not start Purple: {e:#}"));
                    }
                });
            }
        });
    }

    fn champion_list(&mut self, ui: &mut egui::Ui) {
        let query = self.search.to_lowercase();
        let shown: Vec<usize> = (0..self.catalog.champions.len())
            .filter(|&i| {
                let c = &self.catalog.champions[i];
                (query.is_empty() || c.name.to_lowercase().contains(&query))
                    && (self.filter == Filter::All || self.config.skins.contains_key(&c.id))
            })
            .collect();
        if shown.is_empty() {
            ui.add_space(20.0);
            let text = if self.catalog.champions.is_empty() {
                "Loading champions…"
            } else if self.filter == Filter::Chosen && query.is_empty() {
                "No skin chosen yet."
            } else {
                "No champion found."
            };
            ui.label(egui::RichText::new(text).color(color::TEXT_DIM));
            ui.allocate_space(ui.available_size());
            return;
        }
        let mut clicked = None;
        // Rows touch each other; `show_rows` also counts this spacing in its row height.
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut list = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if self.reveal {
            if let Some(index) = shown.iter().position(|&i| Some(self.catalog.champions[i].id) == self.selected) {
                list = list.vertical_scroll_offset((index as f32 - 3.0).max(0.0) * ROW_HEIGHT);
            }
        }
        list.show_rows(ui, ROW_HEIGHT, shown.len(), |ui, range| {
            for &i in &shown[range] {
                let champion = self.catalog.champions[i].clone();
                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
                let selected = self.selected == Some(champion.id);
                let painter = ui.painter().clone();
                let row = rect.shrink2(Vec2::new(0.0, 1.0));
                if selected {
                    painter.rect_filled(row, CornerRadius::same(RADIUS), color::HOVER);
                } else if response.hovered() {
                    painter.rect_filled(row, CornerRadius::same(RADIUS), color::RAISED);
                }
                let portrait = Rect::from_center_size(row.left_center() + Vec2::new(8.0 + PORTRAIT / 2.0, 0.0), Vec2::splat(PORTRAIT));
                match self.images.get(&champion.icon_path, 64) {
                    Some(texture) => {
                        egui::Image::from_texture(texture).corner_radius(CornerRadius::same(5)).paint_at(ui, portrait);
                    }
                    None => {
                        painter.rect_filled(portrait, CornerRadius::same(5), color::RAISED);
                    }
                }
                let text_x = portrait.right() + 10.0;
                let skin = self.config.skins.get(&champion.id).map(|&s| self.skin_name(s));
                let clip = painter.with_clip_rect(row.shrink2(Vec2::new(8.0, 0.0)));
                match skin {
                    Some(skin) => {
                        clip.text(Pos2::new(text_x, row.center().y - 8.0), Align2::LEFT_CENTER, &champion.name, regular(13.5), color::TEXT);
                        clip.text(Pos2::new(text_x, row.center().y + 9.0), Align2::LEFT_CENTER, skin, regular(12.0), color::TEXT_DIM);
                        // Marks the champions that have a skin chosen.
                        painter.circle_filled(Pos2::new(row.right() - 12.0, row.center().y), 3.0, color::ACCENT);
                    }
                    None => {
                        clip.text(Pos2::new(text_x, row.center().y), Align2::LEFT_CENTER, &champion.name, regular(13.5), color::TEXT);
                    }
                }
                if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    clicked = Some(champion.id);
                }
            }
        });
        if clicked.is_some() {
            self.selected = clicked;
            self.reveal = true; // show the chosen skin of the newly picked champion
        }
    }

    // ---- Main area ----------------------------------------------------------------------------

    fn header(&mut self, ui: &mut egui::Ui, champion_id: u32) {
        let Some(champion) = self.catalog.champion(&champion_id.to_string()).cloned() else { return };
        ui.horizontal(|ui| {
            if let Some(texture) = self.images.get(&champion.icon_path, 128) {
                ui.add(egui::Image::from_texture(texture).fit_to_exact_size(Vec2::splat(44.0)).corner_radius(CornerRadius::same(8)));
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(1.0);
                ui.label(egui::RichText::new(&champion.name).font(semibold(22.0)));
                let count = self.catalog.skins_of(champion.id).filter(|s| !s.is_base).count();
                ui.label(egui::RichText::new(format!("{count} skins")).color(color::TEXT_DIM).size(13.0));
            });
        });
    }

    fn notice(ui: &mut egui::Ui, text: &str, tint: Color32) {
        let response = egui::Frame::new()
            .fill(color::RAISED)
            .stroke(Stroke::new(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(RADIUS))
            .inner_margin(Margin { left: 16, right: 12, top: 9, bottom: 9 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(text).color(color::TEXT).size(13.0));
            })
            .response;
        let bar = Rect::from_min_size(response.rect.min + Vec2::new(1.0, 1.0), Vec2::new(3.0, response.rect.height() - 2.0));
        ui.painter().rect_filled(bar, CornerRadius { nw: RADIUS, sw: RADIUS, ne: 0, se: 0 }, tint);
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        if self.catalog.champions.is_empty() {
            empty_state(ui, "Loading champions", "Purple reads them from your League of Legends installation.");
            return;
        }
        let Some(champion_id) = self.selected else {
            empty_state(ui, "No champion selected", "Pick one on the left to see its skins.");
            return;
        };
        self.header(ui, champion_id);
        ui.add_space(14.0);
        if let Some((text, tint)) = self.problem() {
            Self::notice(ui, &text, tint);
            ui.add_space(14.0);
        }

        let chosen = self.config.skins.get(&champion_id).copied();
        let base = self.catalog.skins_of(champion_id).find(|s| s.is_base).cloned();
        let skins: Vec<Skin> = self.catalog.skins_of(champion_id).filter(|s| !s.is_base).cloned().collect();
        let mut new_choice: Option<Option<u32>> = None;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width() - 6.0;
            let columns = (((width + CARD_GAP) / (CARD_MIN_WIDTH + CARD_GAP)).floor() as usize).max(1);
            let card_width = (width - CARD_GAP * (columns - 1) as f32) / columns as f32;
            // The default skin comes first, then every other skin.
            let cards: Vec<(Option<u32>, String, String)> = std::iter::once((
                None,
                "Default".to_owned(),
                base.as_ref().map(|s| s.tile_path.clone()).unwrap_or_default(),
            ))
            .chain(skins.iter().map(|s| (Some(s.id), s.name.clone(), s.tile_path.clone())))
            .collect();
            for row in cards.chunks(columns) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = CARD_GAP;
                    for (id, name, tile) in row {
                        let is_chosen = chosen == *id;
                        let response = self.skin_card(ui, card_width, tile, name, is_chosen);
                        if is_chosen && self.reveal {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() && !is_chosen {
                            new_choice = Some(*id);
                        }
                    }
                });
                ui.add_space(CARD_GAP - 8.0);
            }
            ui.add_space(8.0);
        });
        if let Some(choice) = new_choice {
            self.choose(champion_id, choice);
        }
    }

    /// Draws one card: the skin's picture with its name underneath.
    fn skin_card(&mut self, ui: &mut egui::Ui, width: f32, tile_path: &str, name: &str, chosen: bool) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, width + CARD_LABEL), Sense::click());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let painter = ui.painter().clone();
        let radius = CornerRadius::same(RADIUS);
        let picture = Rect::from_min_size(rect.min, Vec2::splat(width));
        match self.images.get(tile_path, 320) {
            Some(texture) => {
                egui::Image::from_texture(texture).corner_radius(radius).paint_at(ui, picture);
            }
            None => {
                painter.rect_filled(picture, radius, color::RAISED);
            }
        }
        let border = if chosen {
            Stroke::new(2.0, color::ACCENT)
        } else if response.hovered() {
            Stroke::new(1.0, color::BORDER_STRONG.gamma_multiply(1.6))
        } else {
            Stroke::new(1.0, color::BORDER)
        };
        painter.rect_stroke(picture, radius, border, StrokeKind::Outside);
        if chosen {
            check_badge(&painter, picture.right_top() + Vec2::new(-15.0, 15.0));
        }
        let (font, text_color) = if chosen { (semibold(13.0), color::TEXT) } else { (regular(13.0), color::TEXT_DIM) };
        let text_color = if response.hovered() { color::TEXT } else { text_color };
        let galley = painter.layout(name.to_owned(), font, text_color, width);
        painter.with_clip_rect(rect).galley(Pos2::new(rect.left(), picture.bottom() + 8.0), galley, text_color);
        response.on_hover_cursor(CursorIcon::PointingHand)
    }
}

/// A two-way segmented control (with counts) bound to `value`.
fn segmented(ui: &mut egui::Ui, value: &mut Filter, options: [(Filter, &str, usize); 2]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(RADIUS), color::WINDOW, Stroke::new(1.0, color::BORDER), StrokeKind::Inside);
    let half = rect.width() / 2.0;
    for (i, (option, label, count)) in options.into_iter().enumerate() {
        let segment = Rect::from_min_size(rect.min + Vec2::new(half * i as f32, 0.0), Vec2::new(half, rect.height())).shrink(3.0);
        let response = ui.interact(segment, ui.make_persistent_id(("segment", i)), Sense::click());
        let on = *value == option;
        if on {
            ui.painter().rect_filled(segment, CornerRadius::same(RADIUS - 2), color::HOVER);
        }
        let text_color = if on || response.hovered() { color::TEXT } else { color::TEXT_DIM };
        let mut job = egui::text::LayoutJob::default();
        job.append(label, 0.0, egui::TextFormat::simple(regular(12.5), text_color));
        job.append(&count.to_string(), 6.0, egui::TextFormat::simple(regular(12.5), color::TEXT_FAINT));
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(segment.center() - galley.size() / 2.0, galley, text_color);
        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            *value = option;
        }
    }
}

/// A filled accent circle with a check mark.
fn check_badge(painter: &egui::Painter, centre: Pos2) {
    painter.circle_filled(centre, 10.0, color::ACCENT);
    let stroke = Stroke::new(1.8, Color32::WHITE);
    let points = [centre + Vec2::new(-4.0, 0.3), centre + Vec2::new(-1.2, 3.2), centre + Vec2::new(4.2, -2.8)];
    painter.line_segment([points[0], points[1]], stroke);
    painter.line_segment([points[1], points[2]], stroke);
}

fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(ui.available_height() * 0.35);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(title).font(semibold(16.0)));
        ui.label(egui::RichText::new(detail).color(color::TEXT_DIM).size(13.0));
    });
}

/// Development aid: `PURPLE_SCREENSHOT=<file.png>` saves a capture of the window once the
/// pictures have had time to load, then closes it. `PURPLE_SCREENSHOT_CHAMPION=<name>` picks
/// the champion shown.
struct Screenshot {
    path: std::path::PathBuf,
    champion: Option<String>,
    started: Instant,
    requested: bool,
    done: Arc<std::sync::atomic::AtomicBool>,
}

impl Screenshot {
    fn from_env() -> Option<Self> {
        Some(Self {
            path: std::env::var_os("PURPLE_SCREENSHOT")?.into(),
            champion: std::env::var("PURPLE_SCREENSHOT_CHAMPION").ok(),
            started: Instant::now(),
            requested: false,
            done: Arc::default(),
        })
    }

    fn step(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        if self.done.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        if self.requested || self.started.elapsed() < Duration::from_secs(6) {
            return;
        }
        self.requested = true;
        let (path, done, repaint) = (self.path.clone(), self.done.clone(), ctx.clone());
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let [w, h] = image.size;
            let _ = image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8);
            done.store(true, Ordering::Relaxed);
            repaint.request_repaint();
        })));
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_engine();
        while let Ok(catalog) = self.catalog_updates.try_recv() {
            if self.selected.is_none() {
                self.selected = catalog.champions.first().map(|c| c.id);
            }
            self.catalog = catalog;
        }
        if let Some(screenshot) = self.screenshot.as_mut() {
            screenshot.step(ctx);
        }
        // Keep polling the engine's status while the window is open.
        ctx.request_repaint_after(POLL);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        color::WINDOW.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.images.begin_frame(&ctx);

        egui::Panel::left("champions")
            .exact_size(SIDEBAR_WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(color::SIDEBAR).inner_margin(Margin::symmetric(12, 0)))
            .show(ui, |ui| {
                let edge = ui.max_rect().right() + 12.0;
                ui.painter().vline(edge, ui.max_rect().y_range().expand(20.0), Stroke::new(1.0, color::BORDER));
                self.sidebar(ui);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(color::WINDOW).inner_margin(Margin { left: 28, right: 22, top: 22, bottom: 0 }))
            .show(ui, |ui| self.content(ui));

        self.images.end_frame();
        // Keep revealing until the pictures have loaded, so the layout no longer moves.
        if self.reveal && ctx.input(|i| i.time) > 2.0 && self.screenshot.as_ref().is_none_or(|s| s.requested) {
            self.reveal = false;
        }
    }
}
