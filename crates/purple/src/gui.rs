//! The Purple window: pick a skin per champion.
//!
//! It is only a view. The engine runs in `purple.exe` (tray); this window writes the choices to
//! `config.json` and reads what the engine is doing from `status.json`.

use crate::catalog::{Catalog, Skin};
use crate::config::Config;
use crate::icon;
use crate::images::Images;
use crate::lcu::Lcu;
use crate::status::Status;
use crate::win;
use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const PURPLE: Color32 = Color32::from_rgb(139, 92, 246);
const ROW_HEIGHT: f32 = 34.0;
const ICON: f32 = 26.0;
const CARD_WIDTH: f32 = 150.0;
/// Skin tiles are square (380x380 in the client), so the picture area is square too.
const TILE_HEIGHT: f32 = CARD_WIDTH;
const CARD_HEIGHT: f32 = TILE_HEIGHT + 48.0;
const POLL: Duration = Duration::from_millis(500);

struct App {
    ctx: egui::Context,
    catalog_updates: Receiver<Catalog>,
    catalog_sender: Sender<Catalog>,
    catalog: Catalog,
    config: Config,
    images: Images,
    selected: Option<u32>,
    search: String,
    status: Status,
    engine_running: bool,
    last_poll: Option<Instant>,
    error: Option<String>,
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(win::WINDOW_TITLE)
            .with_inner_size([940.0, 600.0])
            .with_min_inner_size([720.0, 420.0])
            .with_icon(egui::IconData { rgba: icon::rgba(64), width: 64, height: 64 }),
        ..Default::default()
    };
    eframe::run_native(win::WINDOW_TITLE, options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = PURPLE;
        visuals.hyperlink_color = PURPLE;
        ctx.set_visuals(visuals);

        let (catalog_sender, catalog_updates) = mpsc::channel();
        let catalog = Catalog::load_cached().unwrap_or_default();
        let config = Config::load().unwrap_or_default();
        let selected = config.skins.keys().next().copied().or_else(|| catalog.champions.first().map(|c| c.id));
        let app = Self {
            ctx: ctx.clone(),
            catalog_updates,
            catalog_sender,
            catalog,
            config,
            images: Images::new(ctx),
            selected,
            search: String::new(),
            status: Status::default(),
            engine_running: false,
            last_poll: None,
            error: None,
        };
        app.refresh_catalog();
        app
    }

    fn champion_name(&self, id: u32) -> String {
        self.catalog.champion(&id.to_string()).map_or_else(|| format!("champion {id}"), |c| c.name.clone())
    }

    fn skin_name(&self, id: u32) -> String {
        self.catalog.skins.iter().find(|s| s.id == id).map_or_else(|| format!("skin {id}"), |s| s.name.clone())
    }

    /// Reloads the catalog from the client in the background (no-op when it is not running).
    fn refresh_catalog(&self) {
        let (sender, ctx) = (self.catalog_sender.clone(), self.ctx.clone());
        thread::spawn(move || {
            let Ok(lcu) = Lcu::connect() else { return };
            if let Ok(catalog) = Catalog::load(&lcu) {
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

    fn status_text(&self) -> String {
        if !self.engine_running {
            return "Engine stopped".into();
        }
        if !self.status.client_up {
            return "Waiting for the League client...".into();
        }
        match self.status.phase.as_str() {
            "ChampSelect" => "Champion select".into(),
            "GameStart" | "InProgress" | "Reconnect" => "In game".into(),
            "Lobby" | "Matchmaking" | "ReadyCheck" => "In lobby".into(),
            _ => "Client connected".into(),
        }
    }

    fn choose(&mut self, champion: u32, skin: Option<u32>) {
        match skin {
            Some(skin) => self.config.skins.insert(champion, skin),
            None => self.config.skins.remove(&champion),
        };
        self.error = match self.config.save() {
            Ok(()) => None,
            Err(e) => Some(format!("could not save the choice: {e:#}")),
        };
    }

    fn top_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Purple").color(PURPLE).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                let up = self.engine_running && self.status.client_up;
                let color = if up { Color32::from_rgb(74, 222, 128) } else { Color32::GRAY };
                ui.painter().circle_filled(dot.center(), 4.5, color);
                ui.label(self.status_text());
                if let Some(armed) = &self.status.armed {
                    ui.separator();
                    let text = format!("Armed: {} ({})", self.skin_name(armed.skin), self.champion_name(armed.champion));
                    ui.label(RichText::new(text).color(PURPLE));
                }
            });
        });
    }

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        let warning = Color32::from_rgb(251, 191, 36);
        ui.horizontal(|ui| {
            if !self.engine_running {
                ui.colored_label(warning, "The Purple engine is not running: no skin will be applied.");
                if ui.button("Start it").clicked() {
                    self.error = win::spawn_daemon().err().map(|e| format!("could not start the engine: {e:#}"));
                }
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::from_rgb(248, 113, 113), error);
                }
            } else if self.status.patcher_outdated {
                ui.colored_label(warning, "The patcher is too old for this game version. Update vendor/ltk-patcher.");
            } else if self.status.overlay_unreadable {
                ui.colored_label(warning, "The game could not read the skin files: no skin will show this game.");
            } else if let Some(error) = self.error.as_ref().or(self.status.error.as_ref()) {
                ui.colored_label(Color32::from_rgb(248, 113, 113), error);
            } else {
                ui.weak("Pick a skin for each champion; Purple applies it when you choose that champion.");
            }
        });
    }

    fn champion_list(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search a champion").desired_width(f32::INFINITY));
        ui.add_space(6.0);
        let query = self.search.to_lowercase();
        let shown: Vec<usize> = (0..self.catalog.champions.len())
            .filter(|&i| query.is_empty() || self.catalog.champions[i].name.to_lowercase().contains(&query))
            .collect();
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_HEIGHT, shown.len(), |ui, range| {
            for &i in &shown[range] {
                let champion = &self.catalog.champions[i];
                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
                let selected = self.selected == Some(champion.id);
                let painter = ui.painter();
                if selected {
                    painter.rect_filled(rect, CornerRadius::same(6), PURPLE.gamma_multiply(0.45));
                } else if response.hovered() {
                    painter.rect_filled(rect, CornerRadius::same(6), ui.visuals().widgets.hovered.bg_fill);
                }
                let icon = Rect::from_min_size(rect.left_center() + Vec2::new(6.0, -ICON / 2.0), Vec2::splat(ICON));
                match self.images.get(&champion.icon_path, 64) {
                    Some(texture) => {
                        painter.image(texture.id(), icon, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(icon, CornerRadius::same(4), Color32::from_gray(40));
                    }
                }
                painter.text(
                    egui::pos2(icon.right() + 10.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    &champion.name,
                    FontId::proportional(15.0),
                    ui.visuals().text_color(),
                );
                if self.config.skins.contains_key(&champion.id) {
                    painter.circle_filled(egui::pos2(rect.right() - 12.0, rect.center().y), 4.0, PURPLE);
                }
                if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    clicked = Some(champion.id);
                }
            }
        });
        if clicked.is_some() {
            self.selected = clicked;
        }
    }

    fn skin_grid(&mut self, ui: &mut egui::Ui) {
        let Some(champion_id) = self.selected else {
            ui.centered_and_justified(|ui| ui.label("Pick a champion on the left."));
            return;
        };
        if self.catalog.champions.is_empty() {
            ui.centered_and_justified(|ui| ui.label("Open the League client once so Purple can load the champions and skins."));
            return;
        }
        let champion = self.catalog.champion(&champion_id.to_string()).cloned();
        let chosen = self.config.skins.get(&champion_id).copied();
        let skins: Vec<Skin> = self.catalog.skins_of(champion_id).filter(|s| !s.is_base).cloned().collect();

        let mut new_choice: Option<Option<u32>> = None;
        ui.horizontal(|ui| {
            if let Some(champion) = &champion {
                if let Some(texture) = self.images.get(&champion.icon_path, 64) {
                    ui.image((texture.id(), Vec2::splat(44.0)));
                }
                ui.heading(&champion.name);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(skin) = chosen {
                    if ui.button("Use the default skin").clicked() {
                        new_choice = Some(None);
                    }
                    ui.label(RichText::new(format!("Chosen: {}", self.skin_name(skin))).color(PURPLE));
                } else {
                    ui.label(RichText::new("No skin chosen: the game keeps the default look.").weak());
                }
            });
        });
        ui.separator();
        if skins.is_empty() {
            ui.label("This champion has no other skins.");
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(12.0);
                for skin in &skins {
                    let is_chosen = chosen == Some(skin.id);
                    if self.skin_card(ui, skin, is_chosen) {
                        new_choice = Some(if is_chosen { None } else { Some(skin.id) });
                    }
                }
            });
        });
        if let Some(choice) = new_choice {
            self.choose(champion_id, choice);
        }
    }

    /// Draws one skin card; returns true when it was clicked.
    fn skin_card(&mut self, ui: &mut egui::Ui, skin: &Skin, chosen: bool) -> bool {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(CARD_WIDTH, CARD_HEIGHT), Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let background = if response.hovered() { Color32::from_gray(38) } else { Color32::from_gray(28) };
            painter.rect_filled(rect, CornerRadius::same(8), background);
            let tile = Rect::from_min_size(rect.min, Vec2::new(CARD_WIDTH, TILE_HEIGHT));
            let top_corners = CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 };
            match self.images.get(&skin.tile_path, 200) {
                Some(texture) => {
                    egui::Image::from_texture(texture).corner_radius(top_corners).paint_at(ui, tile);
                }
                None => {
                    painter.rect_filled(tile, top_corners, Color32::from_gray(45));
                }
            }
            let galley = painter.layout(skin.name.clone(), FontId::proportional(13.0), ui.visuals().text_color(), CARD_WIDTH - 14.0);
            painter.galley(rect.left_top() + Vec2::new(7.0, TILE_HEIGHT + 6.0), galley, Color32::WHITE);
            if chosen {
                painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(3.0, PURPLE), StrokeKind::Inside);
                let badge = Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 8.0), Vec2::new(56.0, 20.0));
                painter.rect_filled(badge, CornerRadius::same(10), PURPLE);
                painter.text(badge.center(), Align2::CENTER_CENTER, "Active", FontId::proportional(12.0), Color32::WHITE);
            }
        }
        response.on_hover_cursor(CursorIcon::PointingHand).clicked()
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
        // Keep polling the engine's status while the window is open.
        ctx.request_repaint_after(POLL);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.images.begin_frame(&ctx);

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("bottom").show(ui, |ui| self.bottom_bar(ui));
        egui::Panel::left("champions").exact_size(250.0).resizable(false).show(ui, |ui| self.champion_list(ui));
        egui::CentralPanel::default().show(ui, |ui| self.skin_grid(ui));

        self.images.end_frame();
    }
}
